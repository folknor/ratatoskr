use std::collections::HashMap;

use common::types::ProviderCtx;
use db::db::DbState;
use store::body_store::BodyStoreState;
use store::inline_image_store::InlineImageStoreState;

use crate::actions::ActionContext;
use crate::actions::pending::enqueue_if_retryable;
use crate::actions::provider::create_provider;
use crate::progress::NoopProgressReporter;

/// Summary data for a chat contact in the sidebar.
#[derive(Debug, Clone)]
pub struct ChatContactSummary {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub latest_message_preview: Option<String>,
    pub latest_message_at: Option<i64>,
    pub unread_count: i64,
    pub sort_order: i64,
}

/// A single message in a chat timeline.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub is_read: bool,
    pub is_from_user: bool,
    /// Plain-text body, loaded from the body store and run through
    /// signature stripping + quote collapsing. `None` if the body store
    /// has no row for this message (shouldn't happen for seeded data).
    pub body_text: Option<String>,
    /// The original plain-text body before stripping. Used to render the
    /// "Show full message" expanded view; identical to `body_text` when
    /// the strippers found nothing to remove.
    pub body_text_full: Option<String>,
    /// Inline image attachments to render alongside the body.
    pub inline_images: Vec<ChatInlineImage>,
}

/// An inline image attachment, ready to hand to `iced::widget::image`.
#[derive(Debug, Clone)]
pub struct ChatInlineImage {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// Designate an email address as a chat contact.
///
/// Inserts into `chat_contacts`, scans existing threads for 1:1 eligibility,
/// sets `is_chat_thread` on qualifying threads, and computes initial summary.
pub async fn designate_chat_contact(
    db: &DbState,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    if user_emails.iter().any(|ue| ue == &email) {
        return Err("Cannot designate your own email address as a chat contact".to_string());
    }

    db.with_conn(move |conn| {
        crate::db::queries_extra::chat::designate_chat_contact_sync(conn, &email, &user_emails)
    })
    .await
}

/// Remove chat contact designation.
///
/// Clears `is_chat_thread` on all affected threads and deletes the contact row.
pub async fn undesignate_chat_contact(db: &DbState, email: &str) -> Result<(), String> {
    let email = email.to_lowercase();

    db.with_conn(move |conn| crate::db::queries_extra::chat::undesignate_chat_contact_sync(conn, &email))
        .await
}

/// Mark every unread message in a contact's chat threads as read.
///
/// Single transaction: flips `messages.is_read`, mirrors on
/// `threads.is_read`, and resets the denormalised
/// `chat_contacts.unread_count`. Returns the `(account_id, thread_id)`
/// pairs that had unread messages, so the caller can dispatch provider
/// mark-read against each of them.
pub async fn mark_chat_read_local(
    db: &DbState,
    email: &str,
) -> Result<Vec<(String, String)>, String> {
    let email = email.to_lowercase();
    db.with_conn(move |conn| {
        crate::db::queries_extra::chat::mark_chat_read_local_sync(conn, &email)
    })
    .await
}

/// Dispatch provider `mark_read(thread, true)` for every affected thread.
///
/// Builds one provider per account, then reuses it across that account's
/// threads. Failures enqueue a pending op for the periodic retry worker.
/// Entering a chat is a navigation side effect, not a user-initiated
/// action - no toasts, no undo, no completion handler. Callers
/// fire-and-forget.
pub async fn mark_chat_read_remote(
    ctx: &ActionContext,
    affected: Vec<(String, String)>,
) {
    if affected.is_empty() {
        return;
    }

    let mut by_account: HashMap<String, Vec<String>> = HashMap::new();
    for (aid, tid) in affected {
        by_account.entry(aid).or_default().push(tid);
    }

    for (account_id, thread_ids) in by_account {
        let provider = match create_provider(&ctx.db, &account_id, ctx.encryption_key).await {
            Ok(p) => p,
            Err(e) => {
                for thread_id in &thread_ids {
                    let outcome = crate::actions::ActionOutcome::LocalOnly {
                        reason: crate::actions::ActionError::remote(e.clone()),
                        retryable: true,
                    };
                    enqueue_if_retryable(
                        ctx,
                        &outcome,
                        &account_id,
                        "markRead",
                        thread_id,
                        r#"{"read":true}"#,
                    )
                    .await;
                }
                continue;
            }
        };

        for thread_id in thread_ids {
            let provider_ctx = ProviderCtx {
                account_id: &account_id,
                db: &ctx.db,
                body_store: &ctx.body_store,
                inline_images: &ctx.inline_images,
                search: &ctx.search,
                progress: &NoopProgressReporter,
            };
            let outcome = match provider.mark_read(&provider_ctx, &thread_id, true).await {
                Ok(()) => crate::actions::ActionOutcome::Success,
                Err(e) => crate::actions::ActionOutcome::LocalOnly {
                    reason: crate::actions::ActionError::remote(e.to_string()),
                    retryable: true,
                },
            };
            enqueue_if_retryable(
                ctx,
                &outcome,
                &account_id,
                "markRead",
                &thread_id,
                r#"{"read":true}"#,
            )
            .await;
        }
    }
}

/// List all chat contacts with sidebar summary data.
pub async fn get_chat_contacts(db: &DbState) -> Result<Vec<ChatContactSummary>, String> {
    db.with_conn(|conn| {
        crate::db::queries_extra::chat::get_chat_contacts_sync(conn).map(|rows| {
            rows.into_iter()
                .map(|row| ChatContactSummary {
                    email: row.email,
                    display_name: row.display_name,
                    avatar_path: row.avatar_path,
                    latest_message_preview: row.latest_message_preview,
                    latest_message_at: row.latest_message_at,
                    unread_count: row.unread_count,
                    sort_order: row.sort_order,
                })
                .collect()
        })
    })
    .await
}

/// Get the chat timeline for a contact - paginated message stream.
///
/// Returns messages across all accounts and threads, ordered chronologically
/// (oldest first). Use `before` timestamp for pagination.
///
/// Bodies are loaded from `body_store`; inline image bytes are resolved via
/// `inline_image_store` so the UI can render them directly.
pub async fn get_chat_timeline(
    db: &DbState,
    body_store: &BodyStoreState,
    inline_image_store: &InlineImageStoreState,
    email: &str,
    user_emails: &[String],
    limit: usize,
    before: Option<(i64, String)>,
) -> Result<Vec<ChatMessage>, String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    // Phase 1: messages + inline-image rows + user-signature texts from
    // main DB. The signatures power Layer-3 of the strip pipeline below.
    let (mut messages, inline_rows, user_signatures) = db
        .with_conn(move |conn| {
            let rows = crate::db::queries_extra::chat::get_chat_timeline_sync(
                conn, &email, limit, before,
            )?;
            let message_ids: Vec<String> =
                rows.iter().map(|r| r.message_id.clone()).collect();
            let images = crate::db::queries_extra::chat::get_chat_inline_images_sync(
                conn,
                &message_ids,
            )?;
            let signatures =
                crate::db::queries_extra::chat::get_user_signature_texts_sync(conn)?;

            let mut messages: Vec<ChatMessage> = rows
                .into_iter()
                .map(|row| {
                    let is_from_user = user_emails
                        .iter()
                        .any(|ue| ue.eq_ignore_ascii_case(&row.from_address));
                    ChatMessage {
                        message_id: row.message_id,
                        account_id: row.account_id,
                        thread_id: row.thread_id,
                        from_address: row.from_address,
                        from_name: row.from_name,
                        date: row.date,
                        subject: row.subject,
                        is_read: row.is_read,
                        is_from_user,
                        body_text: None,
                        body_text_full: None,
                        inline_images: Vec::new(),
                    }
                })
                .collect();
            messages.reverse();
            Ok::<_, String>((messages, images, signatures))
        })
        .await?;

    if messages.is_empty() {
        return Ok(messages);
    }

    // Phase 2: bodies. Plain text only - HTML rendering is a future
    // chat-bubble enhancement; today we render `body_text` directly.
    let message_ids: Vec<String> = messages.iter().map(|m| m.message_id.clone()).collect();
    let bodies = body_store.get_batch(message_ids).await?;
    let mut body_map: HashMap<String, Option<String>> = HashMap::new();
    for body in bodies {
        body_map.insert(body.message_id, body.body_text);
    }

    // Phase 3: inline image bytes from the inline_image store, batched by hash.
    let mut by_message: HashMap<String, Vec<&db::db::queries_extra::chat::DbChatInlineImage>> =
        HashMap::new();
    for row in &inline_rows {
        by_message.entry(row.message_id.clone()).or_default().push(row);
    }
    // `get_batch_sync` expects `(key_for_result_map, lookup_content_hash)`
    // pairs and is keyed by CID for the reading-pane caller. For chat we
    // just want hash -> bytes, so duplicate the hash on both sides.
    let unique_hashes: Vec<(String, String)> = inline_rows
        .iter()
        .map(|r| (r.content_hash.clone(), r.content_hash.clone()))
        .collect();
    let bytes_map = if unique_hashes.is_empty() {
        HashMap::new()
    } else {
        let conn = inline_image_store.conn();
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| format!("inline image store lock: {e}"))?;
            InlineImageStoreState::get_batch_sync(&conn, &unique_hashes)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))??
    };

    // The strip pipeline takes &[&str], so realise borrowing once.
    let user_sig_refs: Vec<&str> = user_signatures.iter().map(String::as_str).collect();

    for msg in &mut messages {
        if let Some(raw) = body_map.remove(&msg.message_id).flatten() {
            // Layer-3 (user signature) only applies to outbound mail.
            let layer3: &[&str] = if msg.is_from_user {
                &user_sig_refs
            } else {
                &[]
            };
            let stripped = chat_strip_body(&raw, layer3);
            msg.body_text = Some(stripped);
            msg.body_text_full = Some(raw);
        }
        if let Some(rows) = by_message.get(&msg.message_id) {
            for row in rows {
                if let Some(bytes) = bytes_map.get(&row.content_hash) {
                    msg.inline_images.push(ChatInlineImage {
                        mime_type: row.mime_type.clone(),
                        bytes: bytes.clone(),
                    });
                }
            }
        }
    }

    Ok(messages)
}

/// Run quote-collapsing then signature-stripping over a plain-text body.
///
/// Falls back to the raw body if stripping leaves nothing - a message
/// that's *entirely* quote + signature is more usefully shown verbatim
/// than as a blank bubble. The caller still keeps the original on
/// `ChatMessage::body_text_full` for the "show full message" toggle.
fn chat_strip_body(raw: &str, user_signatures: &[&str]) -> String {
    let after_quotes = common::signature_strip::collapse_quotes(raw, false);
    let after_sig = common::signature_strip::strip_signature(
        &after_quotes,
        false,
        user_signatures,
    );
    if after_sig.trim().is_empty() {
        raw.to_string()
    } else {
        after_sig
    }
}
