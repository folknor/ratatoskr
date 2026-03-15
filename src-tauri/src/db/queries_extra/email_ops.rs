#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{
    DbLocalDraft, DbSendAsAlias, DbScheduledEmail, DbSignature, DbTemplate,
    ImapMessageRow, SubscriptionEntry,
};

// ── Templates ───────────────────────────────────────────────

db_command! {
    fn db_get_templates_for_account(state, account_id: String) -> Vec<DbTemplate>;
    fn db_insert_template(state, account_id: Option<String>, name: String, subject: Option<String>, body_html: String, shortcut: Option<String>) -> String;
    fn db_update_template(state, id: String, name: Option<String>, subject: Option<String>, subject_set: bool, body_html: Option<String>, shortcut: Option<String>, shortcut_set: bool) -> ();
    fn db_delete_template(state, id: String) -> ();
}

// ── Signatures ──────────────────────────────────────────────

db_command! {
    fn db_get_signatures_for_account(state, account_id: String) -> Vec<DbSignature>;
    fn db_get_default_signature(state, account_id: String) -> Option<DbSignature>;
    fn db_insert_signature(state, account_id: String, name: String, body_html: String, is_default: bool) -> String;
    fn db_update_signature(state, id: String, name: Option<String>, body_html: Option<String>, is_default: Option<bool>) -> ();
    fn db_delete_signature(state, id: String) -> ();
}

// ── Send-As Aliases ─────────────────────────────────────────

db_command! {
    fn db_get_aliases_for_account(state, account_id: String) -> Vec<DbSendAsAlias>;
    fn db_upsert_alias(
        state,
        account_id: String,
        email: String,
        display_name: Option<String>,
        reply_to_address: Option<String>,
        signature_id: Option<String>,
        is_primary: bool,
        is_default: bool,
        treat_as_alias: bool,
        verification_status: String
    ) -> String;
    fn db_get_default_alias(state, account_id: String) -> Option<DbSendAsAlias>;
    fn db_set_default_alias(state, account_id: String, alias_id: String) -> ();
    fn db_delete_alias(state, id: String) -> ();
}

// ── Local Drafts ────────────────────────────────────────────

db_command! {
    fn db_save_local_draft(
        state,
        id: String,
        account_id: String,
        to_addresses: Option<String>,
        cc_addresses: Option<String>,
        bcc_addresses: Option<String>,
        subject: Option<String>,
        body_html: Option<String>,
        reply_to_message_id: Option<String>,
        thread_id: Option<String>,
        from_email: Option<String>,
        signature_id: Option<String>,
        remote_draft_id: Option<String>,
        attachments: Option<String>
    ) -> ();
    fn db_get_local_draft(state, id: String) -> Option<DbLocalDraft>;
    fn db_get_unsynced_drafts(state, account_id: String) -> Vec<DbLocalDraft>;
    fn db_mark_draft_synced(state, id: String, remote_draft_id: String) -> ();
    fn db_delete_local_draft(state, id: String) -> ();
}

// ── Scheduled Emails ────────────────────────────────────────

db_command! {
    fn db_get_pending_scheduled_emails(state, now: i64) -> Vec<DbScheduledEmail>;
    fn db_get_scheduled_emails_for_account(state, account_id: String) -> Vec<DbScheduledEmail>;
    fn db_update_scheduled_email_status(state, id: String, status: String) -> ();
    fn db_delete_scheduled_email(state, id: String) -> ();
    fn db_update_scheduled_email_attachments(state, account_id: String, attachment_data: String) -> ();
}

// db_insert_scheduled_email has custom logic (unwraps delegation), so hand-written.
#[tauri::command]
pub async fn db_insert_scheduled_email(
    state: tauri::State<'_, crate::db::DbState>,
    account_id: String,
    to_addresses: String,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: String,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    scheduled_at: i64,
    signature_id: Option<String>,
    delegation: Option<String>,
    from_email: Option<String>,
    timezone: Option<String>,
) -> Result<String, String> {
    ratatoskr_core::db::queries_extra::db_insert_scheduled_email(
        &state,
        account_id,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        subject,
        body_html,
        reply_to_message_id,
        thread_id,
        scheduled_at,
        signature_id,
        delegation.unwrap_or_else(|| "local".to_string()),
        from_email,
        timezone,
    )
    .await
}

// ── Unsubscribe ─────────────────────────────────────────────

db_command! {
    fn db_record_unsubscribe_action(
        state,
        id: String,
        account_id: String,
        thread_id: String,
        from_address: String,
        from_name: Option<String>,
        method: String,
        unsubscribe_url: String,
        status: String,
        now: i64
    ) -> ();
    fn db_get_subscriptions(state, account_id: String) -> Vec<SubscriptionEntry>;
    fn db_get_unsubscribe_status(state, account_id: String, from_address: String) -> Option<String>;
}

// ── IMAP ────────────────────────────────────────────────────

db_command! {
    fn db_get_imap_uids_for_messages(state, account_id: String, message_ids: Vec<String>) -> Vec<ImapMessageRow>;
    fn db_find_special_folder(state, account_id: String, special_use: String, fallback_label_id: Option<String>) -> Option<String>;
    fn db_update_message_imap_folder(state, account_id: String, message_ids: Vec<String>, new_folder: String) -> ();
}
