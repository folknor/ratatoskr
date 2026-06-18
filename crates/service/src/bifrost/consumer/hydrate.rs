use bifrost_types::{Change, ObjectChangeKind};
use common::types::{FolderKind, LabelKind, MailProviderKind};
use db::db::queries_extra::MessageInsertRow;
use search::SearchDocument;
use serde::{Deserialize, Serialize};
use store::inline_image_store::InlineImage;

use super::BifrostProviderKind;

const SYNTHETIC_OBJECT_PREFIX: &str = "rtsk-synth:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrationOutcome {
    Succeeded,
    SucceededWithDegradedBody,
    /// A definitive hydration failure with no recoverable metadata
    /// (B3-spec 4.1.3). Skipped, recorded, does NOT block the ack.
    Failed,
    /// An object the provider reports as destroyed. This is a deletion,
    /// NOT a hydration failure, so it must not be lumped into `Failed`.
    /// The deletion write path is a B3a gap; for now the change is
    /// skipped without a message-row write and without blocking the ack.
    Deleted,
    Uncertain,
}

#[derive(Debug, Default, Clone)]
pub struct HydrateBatch {
    pub rows: Vec<ConsumerMessageRow>,
    pub failed: usize,
    pub deleted: usize,
    pub uncertain: usize,
    pub blocked: bool,
}

#[derive(Debug, Clone)]
pub struct ConsumerMessageRow {
    pub message: MessageInsertRow,
    pub folders: Vec<FolderKind>,
    pub labels: Vec<LabelKind>,
    pub keywords: Vec<String>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub inline_images: Vec<InlineImage>,
    pub search_document: SearchDocument,
}

impl HydrateBatch {
    #[must_use]
    pub fn from_changes(
        account_id: &str,
        provider: BifrostProviderKind,
        changes: &[Change],
    ) -> Self {
        let mut batch = Self::default();
        for change in changes {
            match hydrate_change_to_message_insert_row(account_id, provider, change) {
                HydratedChange::Message(row, outcome) => match outcome {
                    HydrationOutcome::Succeeded | HydrationOutcome::SucceededWithDegradedBody => {
                        batch.rows.push(*row);
                    }
                    HydrationOutcome::Failed => {
                        batch.failed = batch.failed.saturating_add(1);
                    }
                    HydrationOutcome::Deleted => {
                        batch.deleted = batch.deleted.saturating_add(1);
                    }
                    HydrationOutcome::Uncertain => {
                        batch.uncertain = batch.uncertain.saturating_add(1);
                        batch.blocked = true;
                    }
                },
                HydratedChange::ScopeOnly => {}
            }
        }
        batch
    }
}

#[derive(Debug, Clone)]
pub enum HydratedChange {
    Message(Box<ConsumerMessageRow>, HydrationOutcome),
    ScopeOnly,
}

#[must_use]
pub fn hydrate_change_to_message_insert_row(
    account_id: &str,
    provider: BifrostProviderKind,
    change: &Change,
) -> HydratedChange {
    match change {
        Change::ObjectChange(object) => {
            let outcome = match object.kind {
                ObjectChangeKind::Created | ObjectChangeKind::Updated => {
                    HydrationOutcome::Succeeded
                }
                ObjectChangeKind::Destroyed => HydrationOutcome::Deleted,
                // ObjectChangeKind is #[non_exhaustive]: an unknown kind is
                // an ambiguous outcome, so block the ack and let the engine
                // re-deliver rather than guess.
                _ => HydrationOutcome::Uncertain,
            };
            if let Some(synthetic) = decode_synthetic_message(&object.id.0) {
                return synthetic_to_row(account_id, provider, synthetic, outcome);
            }
            HydratedChange::Message(
                Box::new(ConsumerMessageRow {
                    message: minimal_message_row(account_id, &object.id.0),
                    folders: Vec::new(),
                    labels: Vec::new(),
                    keywords: Vec::new(),
                    body_html: None,
                    body_text: None,
                    inline_images: Vec::new(),
                    search_document: minimal_search_document(account_id, &object.id.0),
                }),
                outcome,
            )
        }
        // ScopeChange (membership add/remove) drives the membership tables,
        // not a message-row write (B3-spec 2.3.1). The full membership
        // hydration is still a gap; for now it carries no message row.
        Change::ScopeChange(_) => HydratedChange::ScopeOnly,
        // `Change` is #[non_exhaustive]; tolerate future variants as
        // membership-only no-ops rather than panicking the drive loop.
        _ => HydratedChange::ScopeOnly,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub from_addr: String,
    #[serde(default)]
    pub to_addrs: Vec<String>,
    #[serde(default)]
    pub folder_ids: Vec<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub raw_body: Vec<u8>,
    #[serde(default)]
    pub degraded_body: bool,
    #[serde(default)]
    pub forced_outcome: Option<SyntheticOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticOutcome {
    Succeeded,
    DegradedBody,
    Failed,
    Uncertain,
}

pub fn encode_synthetic_message(message: &SyntheticMessage) -> Result<String, String> {
    serde_json::to_string(message)
        .map(|json| format!("{SYNTHETIC_OBJECT_PREFIX}{json}"))
        .map_err(|error| format!("encode synthetic bifrost message: {error}"))
}

fn decode_synthetic_message(id: &str) -> Option<SyntheticMessage> {
    let json = id.strip_prefix(SYNTHETIC_OBJECT_PREFIX)?;
    serde_json::from_str(json).ok()
}

fn synthetic_to_row(
    account_id: &str,
    provider: BifrostProviderKind,
    synthetic: SyntheticMessage,
    mut outcome: HydrationOutcome,
) -> HydratedChange {
    if let Some(forced) = &synthetic.forced_outcome {
        outcome = match forced {
            SyntheticOutcome::Succeeded => HydrationOutcome::Succeeded,
            SyntheticOutcome::DegradedBody => HydrationOutcome::SucceededWithDegradedBody,
            SyntheticOutcome::Failed => HydrationOutcome::Failed,
            SyntheticOutcome::Uncertain => HydrationOutcome::Uncertain,
        };
    } else if synthetic.degraded_body {
        outcome = HydrationOutcome::SucceededWithDegradedBody;
    }
    // The degraded-body lane suppresses body hydration regardless of whether
    // it was requested via the `degraded_body` field or a forced
    // `DegradedBody` outcome - both must yield the same metadata-only row so
    // `body_cached` reflects the failure (spec 4.1.3 degraded-body lane).
    let degraded = matches!(outcome, HydrationOutcome::SucceededWithDegradedBody);
    let body_text = if degraded || synthetic.raw_body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&synthetic.raw_body).into_owned())
    };
    let thread_id = synthetic
        .thread_id
        .clone()
        .unwrap_or_else(|| synthetic.id.clone());
    let snippet = body_text
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect::<String>();
    let message = MessageInsertRow {
        id: synthetic.id.clone(),
        account_id: account_id.to_string(),
        thread_id: thread_id.clone(),
        from_address: Some(synthetic.from_addr.clone()),
        from_name: None,
        to_addresses: if synthetic.to_addrs.is_empty() {
            None
        } else {
            Some(synthetic.to_addrs.join(", "))
        },
        cc_addresses: None,
        bcc_addresses: None,
        reply_to: None,
        subject: Some(synthetic.subject.clone()),
        snippet: snippet.clone(),
        date: 1_700_000_000_000,
        is_read: false,
        is_starred: synthetic
            .keywords
            .iter()
            .any(|keyword| keyword == "$flagged"),
        is_replied: false,
        is_forwarded: false,
        raw_size: i64::try_from(synthetic.raw_body.len()).ok(),
        internal_date: Some(1_700_000_000_000),
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        auth_results: None,
        message_id_header: Some(format!("<{}@synthetic.ratatoskr>", synthetic.id)),
        references_header: None,
        in_reply_to_header: None,
        body_cached: body_text.is_some(),
        mdn_requested: false,
        is_reaction: false,
        imap_uid: None,
        imap_folder: None,
        has_meeting_invite: false,
        meeting_invite_method: None,
        meeting_invite_uid: None,
    };
    let folders = synthetic
        .folder_ids
        .iter()
        .filter_map(|id| FolderKind::parse(id, provider.mail_provider_kind()).ok())
        .collect::<Vec<_>>();
    let labels = synthetic
        .label_ids
        .iter()
        .filter_map(|id| LabelKind::parse(id, provider.mail_provider_kind()).ok())
        .collect::<Vec<_>>();
    let inline_images = if degraded || synthetic.raw_body.is_empty() {
        Vec::new()
    } else {
        vec![InlineImage {
            content_hash: format!("bifrost-synth-{}", synthetic.id),
            data: synthetic.raw_body.clone(),
            mime_type: "text/plain".to_string(),
        }]
    };
    let search_document = SearchDocument {
        message_id: synthetic.id.clone(),
        account_id: account_id.to_string(),
        thread_id,
        subject: Some(synthetic.subject),
        from_name: None,
        from_address: Some(synthetic.from_addr),
        to_addresses: if synthetic.to_addrs.is_empty() {
            None
        } else {
            Some(synthetic.to_addrs.join(", "))
        },
        body_text: body_text.clone(),
        snippet: Some(snippet),
        date: 1_700_000_000,
        is_read: false,
        is_starred: synthetic
            .keywords
            .iter()
            .any(|keyword| keyword == "$flagged"),
        has_attachment: false,
        attachments: Vec::new(),
    };
    HydratedChange::Message(
        Box::new(ConsumerMessageRow {
            message,
            folders,
            labels,
            keywords: synthetic.keywords,
            body_html: None,
            body_text,
            inline_images,
            search_document,
        }),
        outcome,
    )
}

impl BifrostProviderKind {
    fn mail_provider_kind(self) -> MailProviderKind {
        match self {
            Self::Gmail => MailProviderKind::Gmail,
            Self::Graph => MailProviderKind::Graph,
            Self::Imap => MailProviderKind::Imap,
            Self::Jmap => MailProviderKind::Jmap,
        }
    }
}

fn minimal_message_row(account_id: &str, id: &str) -> MessageInsertRow {
    MessageInsertRow {
        id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: id.to_string(),
        from_address: None,
        from_name: None,
        to_addresses: None,
        cc_addresses: None,
        bcc_addresses: None,
        reply_to: None,
        subject: None,
        snippet: String::new(),
        date: 0,
        is_read: true,
        is_starred: false,
        is_replied: false,
        is_forwarded: false,
        raw_size: None,
        internal_date: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        auth_results: None,
        message_id_header: None,
        references_header: None,
        in_reply_to_header: None,
        body_cached: false,
        mdn_requested: false,
        is_reaction: false,
        imap_uid: None,
        imap_folder: None,
        has_meeting_invite: false,
        meeting_invite_method: None,
        meeting_invite_uid: None,
    }
}

fn minimal_search_document(account_id: &str, id: &str) -> SearchDocument {
    SearchDocument {
        message_id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: id.to_string(),
        subject: None,
        from_name: None,
        from_address: None,
        to_addresses: None,
        body_text: None,
        snippet: None,
        date: 0,
        is_read: true,
        is_starred: false,
        has_attachment: false,
        attachments: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{HydratedChange, HydrationOutcome, hydrate_change_to_message_insert_row};
    use crate::bifrost::consumer::BifrostProviderKind;
    use bifrost_types::{Change, ObjectChange, ObjectChangeKind, ObjectId, ScopeChange};

    #[test]
    fn hydrate_change_to_message_insert_row_maps_object_create() {
        let change = Change::ObjectChange(ObjectChange {
            id: ObjectId("m1".to_string()),
            kind: ObjectChangeKind::Created,
        });
        match hydrate_change_to_message_insert_row("acc", BifrostProviderKind::Jmap, &change) {
            HydratedChange::Message(row, HydrationOutcome::Succeeded) => {
                assert_eq!(row.message.id, "m1");
                assert_eq!(row.message.account_id, "acc");
                assert_eq!(row.message.thread_id, "m1");
            }
            other => panic!("unexpected hydration result: {other:?}"),
        }
    }

    #[test]
    fn hydrate_change_to_message_insert_row_classifies_destroyed_as_deleted() {
        let change = Change::ObjectChange(ObjectChange {
            id: ObjectId("m1".to_string()),
            kind: ObjectChangeKind::Destroyed,
        });
        match hydrate_change_to_message_insert_row("acc", BifrostProviderKind::Jmap, &change) {
            HydratedChange::Message(_, HydrationOutcome::Deleted) => {}
            other => panic!("expected Deleted outcome, got {other:?}"),
        }
    }

    #[test]
    fn hydrate_batch_taxonomy_drops_deleted_and_does_not_block_ack() {
        let changes = vec![
            Change::ObjectChange(ObjectChange {
                id: ObjectId("keep".to_string()),
                kind: ObjectChangeKind::Created,
            }),
            Change::ObjectChange(ObjectChange {
                id: ObjectId("gone".to_string()),
                kind: ObjectChangeKind::Destroyed,
            }),
        ];
        let batch = super::HydrateBatch::from_changes("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "deleted item must not persist a row");
        assert_eq!(batch.rows[0].message.id, "keep");
        assert_eq!(batch.deleted, 1);
        assert!(!batch.blocked, "a deletion must not block the ack");
    }

    fn synthetic_change(id: &str, forced: super::SyntheticOutcome) -> Change {
        let synthetic = super::SyntheticMessage {
            id: id.to_string(),
            thread_id: None,
            subject: format!("subject {id}"),
            from_addr: "peer@example.com".to_string(),
            to_addrs: vec!["me@example.com".to_string()],
            folder_ids: vec!["INBOX".to_string()],
            label_ids: Vec::new(),
            keywords: Vec::new(),
            raw_body: b"body".to_vec(),
            degraded_body: false,
            forced_outcome: Some(forced),
        };
        let encoded = super::encode_synthetic_message(&synthetic).unwrap();
        Change::ObjectChange(ObjectChange {
            id: ObjectId(encoded),
            kind: ObjectChangeKind::Created,
        })
    }

    /// Spec 4.1.3 / 6.1: the per-item hydration taxonomy. A degraded-body
    /// item persists its metadata row (NOT dropped) with body hydration off;
    /// a Failed item is skipped and does NOT block the ack; an Uncertain
    /// item leaves siblings persisted but sets `blocked` so the ack is held.
    #[test]
    fn hydrate_change_to_message_insert_row_taxonomy_lanes() {
        use super::SyntheticOutcome;

        // Degraded body: metadata row persists, body hydration off.
        match hydrate_change_to_message_insert_row(
            "acc",
            BifrostProviderKind::Jmap,
            &synthetic_change("deg", SyntheticOutcome::DegradedBody),
        ) {
            HydratedChange::Message(row, HydrationOutcome::SucceededWithDegradedBody) => {
                assert_eq!(row.message.id, "deg");
                assert!(!row.message.body_cached, "degraded body is not cached");
                assert!(row.body_text.is_none(), "degraded body text dropped");
            }
            other => panic!("expected degraded-body lane, got {other:?}"),
        }

        // A Failed item is skipped in the batch and does NOT block the ack.
        let changes = vec![
            synthetic_change("ok", SyntheticOutcome::Succeeded),
            synthetic_change("bad", SyntheticOutcome::Failed),
        ];
        let batch = super::HydrateBatch::from_changes("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "Failed item is skipped");
        assert_eq!(batch.rows[0].message.id, "ok");
        assert_eq!(batch.failed, 1);
        assert!(!batch.blocked, "a Failed item must not block the ack");

        // An Uncertain item persists its Succeeded siblings but blocks the ack.
        let changes = vec![
            synthetic_change("sib", SyntheticOutcome::Succeeded),
            synthetic_change("maybe", SyntheticOutcome::Uncertain),
        ];
        let batch = super::HydrateBatch::from_changes("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "siblings persist, Uncertain does not");
        assert_eq!(batch.uncertain, 1);
        assert!(batch.blocked, "an Uncertain item must block the ack");
    }

    #[test]
    fn hydrate_scope_change_is_membership_only() {
        let change = Change::ScopeChange(ScopeChange {
            id: ObjectId("m1".to_string()),
            membership: bifrost_types::MembershipScope::Folder(bifrost_types::FolderId(
                "INBOX".to_string(),
            )),
            kind: bifrost_types::ScopeChangeKind::Added,
        });
        assert!(matches!(
            hydrate_change_to_message_insert_row("acc", BifrostProviderKind::Jmap, &change),
            HydratedChange::ScopeOnly
        ));
    }
}
