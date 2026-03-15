#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{
    ContactAttachmentRow, ContactStats, DbContact, DbContactGroup, DbContactGroupMember,
    RecentThread, SameDomainContact,
};

db_command! {
    fn db_get_all_contacts(state, limit: Option<i64>, offset: Option<i64>) -> Vec<DbContact>;
    fn db_upsert_contact(state, id: String, email: String, display_name: Option<String>) -> ();
    fn db_update_contact(state, id: String, display_name: Option<String>) -> ();
    fn db_update_contact_notes(state, email: String, notes: Option<String>) -> ();
    fn db_delete_contact(state, id: String) -> ();
    fn db_get_contact_stats(state, email: String) -> ContactStats;
    fn db_get_contacts_from_same_domain(state, email: String, limit: Option<i64>) -> Vec<SameDomainContact>;
    fn db_get_latest_auth_result(state, email: String) -> Option<String>;
    fn db_get_recent_threads_with_contact(state, email: String, limit: Option<i64>) -> Vec<RecentThread>;
    fn db_get_attachments_from_contact(state, email: String, limit: Option<i64>) -> Vec<ContactAttachmentRow>;
    fn db_update_contact_avatar(state, email: String, avatar_url: String) -> ();
}

// ── Contact Groups ──────────────────────────────────────────

db_command! {
    fn db_create_contact_group(state, id: String, name: String) -> ();
    fn db_update_contact_group(state, id: String, name: String) -> ();
    fn db_delete_contact_group(state, id: String) -> ();
    fn db_get_all_contact_groups(state) -> Vec<DbContactGroup>;
    fn db_get_contact_group(state, id: String) -> DbContactGroup;
    fn db_get_contact_group_members(state, group_id: String) -> Vec<DbContactGroupMember>;
    fn db_add_contact_group_member(state, group_id: String, member_type: String, member_value: String) -> ();
    fn db_remove_contact_group_member(state, group_id: String, member_type: String, member_value: String) -> ();
    fn db_expand_contact_group(state, group_id: String) -> Vec<String>;
}

// db_search_contact_groups has custom logic (unwraps limit), so it stays hand-written.
#[tauri::command]
pub async fn db_search_contact_groups(
    state: tauri::State<'_, crate::db::DbState>,
    query: String,
    limit: Option<i64>,
) -> Result<Vec<DbContactGroup>, String> {
    ratatoskr_core::db::queries_extra::db_search_contact_groups(
        &state,
        query,
        limit.unwrap_or(10),
    )
    .await
}
