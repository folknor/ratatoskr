#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{LabelSortOrderItem, SnoozedThread};

db_command! {
    fn db_upsert_thread(
        state,
        id: String,
        account_id: String,
        subject: Option<String>,
        snippet: Option<String>,
        last_message_at: Option<i64>,
        message_count: i64,
        is_read: bool,
        is_starred: bool,
        is_important: bool,
        has_attachments: bool
    ) -> ();
    fn db_set_thread_labels(state, account_id: String, thread_id: String, label_ids: Vec<String>) -> ();
    fn db_delete_all_threads_for_account(state, account_id: String) -> ();
    fn db_get_muted_thread_ids(state, account_id: String) -> Vec<String>;
    fn db_get_unread_inbox_count(state) -> i64;
    fn db_get_snoozed_threads_due(state, now: i64) -> Vec<SnoozedThread>;
}

// ── Messages ────────────────────────────────────────────────

db_command! {
    fn db_get_messages_by_ids(state, account_id: String, message_ids: Vec<String>) -> Vec<crate::db::types::DbMessage>;
    fn db_upsert_message(
        state,
        id: String,
        account_id: String,
        thread_id: String,
        from_address: Option<String>,
        from_name: Option<String>,
        to_addresses: Option<String>,
        cc_addresses: Option<String>,
        bcc_addresses: Option<String>,
        reply_to: Option<String>,
        subject: Option<String>,
        snippet: Option<String>,
        date: i64,
        is_read: bool,
        is_starred: bool,
        body_cached: bool,
        raw_size: Option<i64>,
        internal_date: Option<i64>,
        list_unsubscribe: Option<String>,
        list_unsubscribe_post: Option<String>,
        auth_results: Option<String>,
        message_id_header: Option<String>,
        references_header: Option<String>,
        in_reply_to_header: Option<String>,
        imap_uid: Option<i64>,
        imap_folder: Option<String>
    ) -> ();
    fn db_delete_message(state, account_id: String, message_id: String) -> ();
    fn db_update_message_thread_ids(state, account_id: String, message_ids: Vec<String>, thread_id: String) -> ();
    fn db_delete_all_messages_for_account(state, account_id: String) -> ();
    fn db_get_recent_sent_messages(state, account_id: String, account_email: String, limit: Option<i64>) -> Vec<crate::db::types::DbMessage>;
}

// ── Labels ──────────────────────────────────────────────────

db_command! {
    fn db_upsert_label_coalesce(
        state,
        id: String,
        account_id: String,
        name: String,
        label_type: String,
        color_bg: Option<String>,
        color_fg: Option<String>,
        imap_folder_path: Option<String>,
        imap_special_use: Option<String>
    ) -> ();
    fn db_delete_labels_for_account(state, account_id: String) -> ();
    fn db_update_label_sort_order(state, account_id: String, label_orders: Vec<LabelSortOrderItem>) -> ();
}

// ── Attachments ─────────────────────────────────────────────

use crate::db::types::{AttachmentSender, AttachmentWithContext};

db_command! {
    fn db_upsert_attachment(
        state,
        id: String,
        message_id: String,
        account_id: String,
        filename: Option<String>,
        mime_type: Option<String>,
        size: Option<i64>,
        attachment_id: Option<String>,
        content_id: Option<String>,
        is_inline: bool
    ) -> ();
    fn db_get_attachments_for_account(state, account_id: String, limit: i64, offset: i64) -> Vec<AttachmentWithContext>;
    fn db_get_attachment_senders(state, account_id: String) -> Vec<AttachmentSender>;
}
