use crate::provider::text::truncate_graphemes;
use crate::sync::types::MessageMeta;
use crate::threading::ThreadableMessage;

use super::folder_mapper::get_labels_for_message;
use super::types::ImapMessage;

/// Converted message data ready for DB insertion.
pub struct ConvertedMessage {
    /// Local message ID: `imap-{accountId}-{folder}-{uid}`
    pub id: String,
    pub meta: MessageMeta,
    pub threadable: ThreadableMessage,
    /// The original IMAP message (for DB fields like body, attachments, headers).
    pub imap_msg: ImapMessage,
    /// Label IDs for this message in this folder.
    pub label_ids: Vec<String>,
}

/// Generate a synthetic Message-ID for messages that lack one.
fn synthetic_message_id(account_id: &str, folder: &str, uid: u32) -> String {
    format!("synthetic-{account_id}-{folder}-{uid}@ratatoskr.local")
}

/// Convert an IMAP message to the format needed for DB storage and threading.
///
/// `date` field: The IMAP message has `date` in seconds. The TS code stores
/// `date * 1000` (milliseconds) in ParsedMessage. We follow the same convention
/// here so the DB data is consistent.
pub fn convert_imap_message(
    msg: ImapMessage,
    account_id: &str,
    folder_label_id: &str,
) -> ConvertedMessage {
    let local_id = format!("imap-{}-{}-{}", account_id, msg.folder, msg.uid);
    let rfc_message_id = msg
        .message_id
        .clone()
        .unwrap_or_else(|| synthetic_message_id(account_id, &msg.folder, msg.uid));

    let label_ids =
        get_labels_for_message(folder_label_id, msg.is_read, msg.is_starred, msg.is_draft);

    let snippet = msg.snippet.clone().unwrap_or_else(|| {
        msg.body_text
            .as_deref()
            .map(|text| truncate_graphemes(text, 200))
            .unwrap_or_default()
    });

    let has_attachments = !msg.attachments.is_empty();
    // TS stores date in milliseconds
    let date_ms = msg.date * 1000;

    let meta = MessageMeta {
        id: local_id.clone(),
        rfc_message_id: rfc_message_id.clone(),
        label_ids: label_ids.clone(),
        is_read: msg.is_read,
        is_starred: msg.is_starred,
        has_attachments,
        subject: msg.subject.clone(),
        snippet: snippet.clone(),
        date: date_ms,
    };

    let threadable = ThreadableMessage {
        id: local_id.clone(),
        message_id: rfc_message_id,
        in_reply_to: msg.in_reply_to.clone(),
        references: msg.references.clone(),
        subject: msg.subject.clone(),
        date: date_ms,
    };

    ConvertedMessage {
        id: local_id,
        meta,
        threadable,
        imap_msg: msg,
        label_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(uid: u32, folder: &str) -> ImapMessage {
        ImapMessage {
            uid,
            folder: folder.to_string(),
            message_id: Some(format!("<test-{uid}@example.com>")),
            in_reply_to: None,
            references: None,
            from_address: Some("alice@example.com".to_string()),
            from_name: Some("Alice".to_string()),
            to_addresses: Some("bob@example.com".to_string()),
            cc_addresses: None,
            bcc_addresses: None,
            reply_to: None,
            subject: Some("Test Subject".to_string()),
            date: 1700000000,
            is_read: false,
            is_starred: true,
            is_draft: false,
            body_html: Some("<p>Hello</p>".to_string()),
            body_text: Some("Hello".to_string()),
            snippet: None,
            raw_size: 1024,
            list_unsubscribe: None,
            list_unsubscribe_post: None,
            auth_results: None,
            attachments: vec![],
            mdn_requested: false,
        }
    }

    #[test]
    fn test_convert_basic() {
        let msg = make_msg(42, "INBOX");
        let result = convert_imap_message(msg, "acc-1", "INBOX");

        assert_eq!(result.id, "imap-acc-1-INBOX-42");
        assert_eq!(result.meta.date, 1700000000 * 1000);
        assert!(result.meta.label_ids.contains(&"INBOX".to_string()));
        assert!(result.meta.label_ids.contains(&"UNREAD".to_string()));
        assert!(result.meta.label_ids.contains(&"STARRED".to_string()));
        assert_eq!(result.threadable.message_id, "<test-42@example.com>");
    }

    #[test]
    fn test_synthetic_message_id() {
        let mut msg = make_msg(99, "Sent");
        msg.message_id = None;
        let result = convert_imap_message(msg, "acc-2", "SENT");

        assert!(result.threadable.message_id.starts_with("synthetic-"));
        assert!(result.threadable.message_id.contains("acc-2"));
    }

    #[test]
    fn test_snippet_from_body_text() {
        let mut msg = make_msg(1, "INBOX");
        msg.snippet = None;
        msg.body_text = Some("This is the body text for snippet generation".to_string());
        let result = convert_imap_message(msg, "acc-1", "INBOX");
        assert_eq!(
            result.meta.snippet,
            "This is the body text for snippet generation"
        );
    }
}
