use db::db::DbState;

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
pub async fn get_chat_timeline(
    db: &DbState,
    email: &str,
    user_emails: &[String],
    limit: usize,
    before: Option<(i64, String)>,
) -> Result<Vec<ChatMessage>, String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    db.with_conn(move |conn| {
        let mut messages = crate::db::queries_extra::chat::get_chat_timeline_sync(
            conn, &email, limit, before,
        )?
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
            }
        })
        .collect::<Vec<_>>();

        messages.reverse();
        Ok(messages)
    })
    .await
}
