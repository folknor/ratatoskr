// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::DbState;
use super::types::{
    AttachmentSender, AttachmentWithContext, BackfillRow, BundleSummary, BundleSummarySingle,
    CachedAttachmentRow, ContactAttachmentRow, ContactStats, DbAccount, DbAllowlistEntry,
    DbBundleRule, DbCalendar, DbCalendarEvent, DbContact, DbFilterRule, DbFolderSyncState,
    DbFollowUpReminder, DbLocalDraft, DbNotificationVip, DbPhishingAllowlistEntry, DbQuickStep,
    DbScheduledEmail, DbSendAsAlias, DbSignature, DbSmartFolder, DbSmartLabelRule, DbTask,
    DbTaskTag, DbTemplate, DbWritingStyleProfile, ImapMessageRow, LabelSortOrderItem, RecentThread,
    SameDomainContact, SnoozedThread, SortOrderItem, SubscriptionEntry, ThreadCategoryWithManual,
    ThreadInfoRow, TriggeredFollowUp, UncachedAttachment,
};
pub use ratatoskr_core::db::queries_extra::load_recent_rule_categorized_threads;

#[tauri::command]
pub async fn db_get_all_contacts(
    state: State<'_, DbState>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbContact>, String> {
    ratatoskr_core::db::queries_extra::db_get_all_contacts(&state, limit, offset).await
}

#[tauri::command]
pub async fn db_upsert_contact(
    state: State<'_, DbState>,
    id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_contact(&state, id, email, display_name).await
}

#[tauri::command]
pub async fn db_update_contact(
    state: State<'_, DbState>,
    id: String,
    display_name: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_contact(&state, id, display_name).await
}

#[tauri::command]
pub async fn db_update_contact_notes(
    state: State<'_, DbState>,
    email: String,
    notes: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_contact_notes(&state, email, notes).await
}

#[tauri::command]
pub async fn db_delete_contact(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_contact(&state, id).await
}

#[tauri::command]
pub async fn db_get_contact_stats(
    state: State<'_, DbState>,
    email: String,
) -> Result<ContactStats, String> {
    ratatoskr_core::db::queries_extra::db_get_contact_stats(&state, email).await
}

#[tauri::command]
pub async fn db_get_contacts_from_same_domain(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<SameDomainContact>, String> {
    ratatoskr_core::db::queries_extra::db_get_contacts_from_same_domain(&state, email, limit).await
}

#[tauri::command]
pub async fn db_get_latest_auth_result(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_latest_auth_result(&state, email).await
}

#[tauri::command]
pub async fn db_get_recent_threads_with_contact(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<RecentThread>, String> {
    ratatoskr_core::db::queries_extra::db_get_recent_threads_with_contact(&state, email, limit)
        .await
}

#[tauri::command]
pub async fn db_get_attachments_from_contact(
    state: State<'_, DbState>,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<ContactAttachmentRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_attachments_from_contact(&state, email, limit).await
}

#[tauri::command]
pub async fn db_get_filters_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbFilterRule>, String> {
    ratatoskr_core::db::queries_extra::db_get_filters_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_insert_filter(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    name: String,
    criteria_json: String,
    actions_json: String,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_filter(
        &state,
        id,
        account_id,
        name,
        criteria_json,
        actions_json,
        is_enabled,
    )
    .await
}

#[tauri::command]
pub async fn db_update_filter(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    criteria_json: Option<String>,
    actions_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_filter(
        &state,
        id,
        name,
        criteria_json,
        actions_json,
        is_enabled,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_filter(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_filter(&state, id).await
}

#[tauri::command]
pub async fn db_get_smart_folders(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<Vec<DbSmartFolder>, String> {
    ratatoskr_core::db::queries_extra::db_get_smart_folders(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_smart_folder_by_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbSmartFolder>, String> {
    ratatoskr_core::db::queries_extra::db_get_smart_folder_by_id(&state, id).await
}

#[tauri::command]
pub async fn db_insert_smart_folder(
    state: State<'_, DbState>,
    id: String,
    name: String,
    query: String,
    account_id: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_smart_folder(
        &state, id, name, query, account_id, icon, color,
    )
    .await
}

#[tauri::command]
pub async fn db_update_smart_folder(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    query: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_smart_folder(&state, id, name, query, icon, color)
        .await
}

#[tauri::command]
pub async fn db_delete_smart_folder(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_smart_folder(&state, id).await
}

#[tauri::command]
pub async fn db_update_smart_folder_sort_order(
    state: State<'_, DbState>,
    orders: Vec<SortOrderItem>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_smart_folder_sort_order(&state, orders).await
}

#[tauri::command]
pub async fn db_get_smart_label_rules_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSmartLabelRule>, String> {
    ratatoskr_core::db::queries_extra::db_get_smart_label_rules_for_account(&state, account_id)
        .await
}

#[tauri::command]
pub async fn db_insert_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    label_id: String,
    ai_description: String,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_smart_label_rule(
        &state,
        id,
        account_id,
        label_id,
        ai_description,
        criteria_json,
        is_enabled,
    )
    .await
}

#[tauri::command]
pub async fn db_update_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
    label_id: Option<String>,
    ai_description: Option<String>,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_smart_label_rule(
        &state,
        id,
        label_id,
        ai_description,
        criteria_json,
        is_enabled,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_smart_label_rule(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_smart_label_rule(&state, id).await
}

#[tauri::command]
pub async fn db_insert_follow_up_reminder(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    thread_id: String,
    message_id: String,
    remind_at: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_follow_up_reminder(
        &state, id, account_id, thread_id, message_id, remind_at,
    )
    .await
}

#[tauri::command]
pub async fn db_get_follow_up_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbFollowUpReminder>, String> {
    ratatoskr_core::db::queries_extra::db_get_follow_up_for_thread(&state, account_id, thread_id)
        .await
}

#[tauri::command]
pub async fn db_cancel_follow_up_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_cancel_follow_up_for_thread(&state, account_id, thread_id)
        .await
}

#[tauri::command]
pub async fn db_get_active_follow_up_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_active_follow_up_thread_ids(
        &state, account_id, thread_ids,
    )
    .await
}

#[tauri::command]
pub async fn db_check_follow_up_reminders(
    state: State<'_, DbState>,
) -> Result<Vec<TriggeredFollowUp>, String> {
    ratatoskr_core::db::queries_extra::db_check_follow_up_reminders(&state).await
}

#[tauri::command]
pub async fn db_get_quick_steps_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    ratatoskr_core::db::queries_extra::db_get_quick_steps_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_enabled_quick_steps_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    ratatoskr_core::db::queries_extra::db_get_enabled_quick_steps_for_account(&state, account_id)
        .await
}

#[tauri::command]
pub async fn db_insert_quick_step(
    state: State<'_, DbState>,
    step: DbQuickStep,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_quick_step(&state, step).await
}

#[tauri::command]
pub async fn db_update_quick_step(
    state: State<'_, DbState>,
    step: DbQuickStep,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_quick_step(&state, step).await
}

#[tauri::command]
pub async fn db_delete_quick_step(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_quick_step(&state, id).await
}

#[tauri::command]
pub async fn db_add_to_allowlist(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_add_to_allowlist(&state, id, account_id, sender_address)
        .await
}

#[tauri::command]
pub async fn db_get_allowlisted_senders(
    state: State<'_, DbState>,
    account_id: String,
    sender_addresses: Vec<String>,
) -> Result<Vec<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_allowlisted_senders(
        &state,
        account_id,
        sender_addresses,
    )
    .await
}

#[tauri::command]
pub async fn db_add_vip_sender(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_add_vip_sender(
        &state,
        id,
        account_id,
        email,
        display_name,
    )
    .await
}

#[tauri::command]
pub async fn db_remove_vip_sender(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_remove_vip_sender(&state, account_id, email).await
}

#[tauri::command]
pub async fn db_is_vip_sender(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
) -> Result<bool, String> {
    ratatoskr_core::db::queries_extra::db_is_vip_sender(&state, account_id, email).await
}

#[tauri::command]
pub async fn db_set_thread_category(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    category: String,
    is_manual: bool,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_thread_category(
        &state, account_id, thread_id, category, is_manual,
    )
    .await
}

#[tauri::command]
pub async fn db_get_bundle_rules(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbBundleRule>, String> {
    ratatoskr_core::db::queries_extra::db_get_bundle_rules(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_bundle_summaries(
    state: State<'_, DbState>,
    account_id: String,
    categories: Vec<String>,
) -> Result<Vec<BundleSummary>, String> {
    ratatoskr_core::db::queries_extra::db_get_bundle_summaries(&state, account_id, categories).await
}

#[tauri::command]
pub async fn db_get_held_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_held_thread_ids(&state, account_id).await
}

#[tauri::command]
pub async fn db_attachment_cache_total_size(state: State<'_, DbState>) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_attachment_cache_total_size(&state).await
}

#[tauri::command]
pub async fn db_uncached_recent_attachments(
    state: State<'_, DbState>,
    max_size: i64,
    cutoff_epoch: i64,
    limit: i64,
) -> Result<Vec<UncachedAttachment>, String> {
    ratatoskr_core::db::queries_extra::db_uncached_recent_attachments(
        &state,
        max_size,
        cutoff_epoch,
        limit,
    )
    .await
}

#[tauri::command]
pub async fn db_get_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_ai_cache(&state, account_id, thread_id, cache_type)
        .await
}

#[tauri::command]
pub async fn db_set_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
    content: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_ai_cache(
        &state, account_id, thread_id, cache_type, content,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_ai_cache(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_ai_cache(&state, account_id, thread_id, cache_type)
        .await
}

#[tauri::command]
pub async fn db_get_cached_scan_result(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_cached_scan_result(&state, account_id, message_id)
        .await
}

#[tauri::command]
pub async fn db_cache_scan_result(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
    result_json: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_cache_scan_result(
        &state,
        account_id,
        message_id,
        result_json,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_scan_results(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_scan_results(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbWritingStyleProfile>, String> {
    ratatoskr_core::db::queries_extra::db_get_writing_style_profile(&state, account_id).await
}

#[tauri::command]
pub async fn db_upsert_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
    profile_text: String,
    sample_count: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_writing_style_profile(
        &state,
        account_id,
        profile_text,
        sample_count,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_writing_style_profile(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_writing_style_profile(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
) -> Result<Option<DbFolderSyncState>, String> {
    ratatoskr_core::db::queries_extra::db_get_folder_sync_state(&state, account_id, folder_path)
        .await
}

#[tauri::command]
pub async fn db_upsert_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
    uidvalidity: Option<i64>,
    last_uid: i64,
    modseq: Option<i64>,
    last_sync_at: Option<i64>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_folder_sync_state(
        &state,
        account_id,
        folder_path,
        uidvalidity,
        last_uid,
        modseq,
        last_sync_at,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_folder_sync_state(
    state: State<'_, DbState>,
    account_id: String,
    folder_path: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_folder_sync_state(&state, account_id, folder_path)
        .await
}

#[tauri::command]
pub async fn db_clear_all_folder_sync_states(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_clear_all_folder_sync_states(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_all_folder_sync_states(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbFolderSyncState>, String> {
    ratatoskr_core::db::queries_extra::db_get_all_folder_sync_states(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_vip_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_vip_senders(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_all_vip_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbNotificationVip>, String> {
    ratatoskr_core::db::queries_extra::db_get_all_vip_senders(&state, account_id).await
}

#[tauri::command]
pub async fn db_is_allowlisted(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    ratatoskr_core::db::queries_extra::db_is_allowlisted(&state, account_id, sender_address).await
}

#[tauri::command]
pub async fn db_remove_from_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_remove_from_allowlist(&state, account_id, sender_address)
        .await
}

#[tauri::command]
pub async fn db_get_allowlist_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbAllowlistEntry>, String> {
    ratatoskr_core::db::queries_extra::db_get_allowlist_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_is_phishing_allowlisted(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<bool, String> {
    ratatoskr_core::db::queries_extra::db_is_phishing_allowlisted(
        &state,
        account_id,
        sender_address,
    )
    .await
}

#[tauri::command]
pub async fn db_add_to_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_add_to_phishing_allowlist(
        &state,
        account_id,
        sender_address,
    )
    .await
}

#[tauri::command]
pub async fn db_remove_from_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
    sender_address: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_remove_from_phishing_allowlist(
        &state,
        account_id,
        sender_address,
    )
    .await
}

#[tauri::command]
pub async fn db_get_phishing_allowlist(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbPhishingAllowlistEntry>, String> {
    ratatoskr_core::db::queries_extra::db_get_phishing_allowlist(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_templates_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbTemplate>, String> {
    ratatoskr_core::db::queries_extra::db_get_templates_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_insert_template(
    state: State<'_, DbState>,
    account_id: Option<String>,
    name: String,
    subject: Option<String>,
    body_html: String,
    shortcut: Option<String>,
) -> Result<String, String> {
    ratatoskr_core::db::queries_extra::db_insert_template(
        &state, account_id, name, subject, body_html, shortcut,
    )
    .await
}

#[tauri::command]
pub async fn db_update_template(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    subject: Option<String>,
    subject_set: bool,
    body_html: Option<String>,
    shortcut: Option<String>,
    shortcut_set: bool,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_template(
        &state,
        id,
        name,
        subject,
        subject_set,
        body_html,
        shortcut,
        shortcut_set,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_template(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_template(&state, id).await
}

#[tauri::command]
pub async fn db_get_signatures_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSignature>, String> {
    ratatoskr_core::db::queries_extra::db_get_signatures_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_default_signature(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbSignature>, String> {
    ratatoskr_core::db::queries_extra::db_get_default_signature(&state, account_id).await
}

#[tauri::command]
pub async fn db_insert_signature(
    state: State<'_, DbState>,
    account_id: String,
    name: String,
    body_html: String,
    is_default: bool,
) -> Result<String, String> {
    ratatoskr_core::db::queries_extra::db_insert_signature(
        &state, account_id, name, body_html, is_default,
    )
    .await
}

#[tauri::command]
pub async fn db_update_signature(
    state: State<'_, DbState>,
    id: String,
    name: Option<String>,
    body_html: Option<String>,
    is_default: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_signature(&state, id, name, body_html, is_default)
        .await
}

#[tauri::command]
pub async fn db_delete_signature(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_signature(&state, id).await
}

#[tauri::command]
pub async fn db_get_aliases_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbSendAsAlias>, String> {
    ratatoskr_core::db::queries_extra::db_get_aliases_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_upsert_alias(
    state: State<'_, DbState>,
    account_id: String,
    email: String,
    display_name: Option<String>,
    reply_to_address: Option<String>,
    signature_id: Option<String>,
    is_primary: bool,
    is_default: bool,
    treat_as_alias: bool,
    verification_status: String,
) -> Result<String, String> {
    ratatoskr_core::db::queries_extra::db_upsert_alias(
        &state,
        account_id,
        email,
        display_name,
        reply_to_address,
        signature_id,
        is_primary,
        is_default,
        treat_as_alias,
        verification_status,
    )
    .await
}

#[tauri::command]
pub async fn db_get_default_alias(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Option<DbSendAsAlias>, String> {
    ratatoskr_core::db::queries_extra::db_get_default_alias(&state, account_id).await
}

#[tauri::command]
pub async fn db_set_default_alias(
    state: State<'_, DbState>,
    account_id: String,
    alias_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_default_alias(&state, account_id, alias_id).await
}

#[tauri::command]
pub async fn db_delete_alias(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_alias(&state, id).await
}

#[tauri::command]
pub async fn db_save_local_draft(
    state: State<'_, DbState>,
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
    attachments: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_save_local_draft(
        &state,
        id,
        account_id,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        subject,
        body_html,
        reply_to_message_id,
        thread_id,
        from_email,
        signature_id,
        remote_draft_id,
        attachments,
    )
    .await
}

#[tauri::command]
pub async fn db_get_local_draft(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbLocalDraft>, String> {
    ratatoskr_core::db::queries_extra::db_get_local_draft(&state, id).await
}

#[tauri::command]
pub async fn db_get_unsynced_drafts(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbLocalDraft>, String> {
    ratatoskr_core::db::queries_extra::db_get_unsynced_drafts(&state, account_id).await
}

#[tauri::command]
pub async fn db_mark_draft_synced(
    state: State<'_, DbState>,
    id: String,
    remote_draft_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_mark_draft_synced(&state, id, remote_draft_id).await
}

#[tauri::command]
pub async fn db_delete_local_draft(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_local_draft(&state, id).await
}

#[tauri::command]
pub async fn db_get_pending_scheduled_emails(
    state: State<'_, DbState>,
    now: i64,
) -> Result<Vec<DbScheduledEmail>, String> {
    ratatoskr_core::db::queries_extra::db_get_pending_scheduled_emails(&state, now).await
}

#[tauri::command]
pub async fn db_get_scheduled_emails_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbScheduledEmail>, String> {
    ratatoskr_core::db::queries_extra::db_get_scheduled_emails_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_insert_scheduled_email(
    state: State<'_, DbState>,
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
    )
    .await
}

#[tauri::command]
pub async fn db_update_scheduled_email_status(
    state: State<'_, DbState>,
    id: String,
    status: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_scheduled_email_status(&state, id, status).await
}

#[tauri::command]
pub async fn db_delete_scheduled_email(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_scheduled_email(&state, id).await
}

#[tauri::command]
pub async fn db_upsert_label_coalesce(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    name: String,
    label_type: String,
    color_bg: Option<String>,
    color_fg: Option<String>,
    imap_folder_path: Option<String>,
    imap_special_use: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_label_coalesce(
        &state,
        id,
        account_id,
        name,
        label_type,
        color_bg,
        color_fg,
        imap_folder_path,
        imap_special_use,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_labels_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_labels_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_update_label_sort_order(
    state: State<'_, DbState>,
    account_id: String,
    label_orders: Vec<LabelSortOrderItem>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_label_sort_order(&state, account_id, label_orders)
        .await
}

#[tauri::command]
pub async fn db_upsert_attachment(
    state: State<'_, DbState>,
    id: String,
    message_id: String,
    account_id: String,
    filename: Option<String>,
    mime_type: Option<String>,
    size: Option<i64>,
    attachment_id: Option<String>,
    content_id: Option<String>,
    is_inline: bool,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_attachment(
        &state,
        id,
        message_id,
        account_id,
        filename,
        mime_type,
        size,
        attachment_id,
        content_id,
        is_inline,
    )
    .await
}

#[tauri::command]
pub async fn db_get_attachments_for_account(
    state: State<'_, DbState>,
    account_id: String,
    limit: i64,
    offset: i64,
) -> Result<Vec<AttachmentWithContext>, String> {
    ratatoskr_core::db::queries_extra::db_get_attachments_for_account(
        &state, account_id, limit, offset,
    )
    .await
}

#[tauri::command]
pub async fn db_get_attachment_senders(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<AttachmentSender>, String> {
    ratatoskr_core::db::queries_extra::db_get_attachment_senders(&state, account_id).await
}

#[tauri::command]
pub async fn db_update_contact_avatar(
    state: State<'_, DbState>,
    email: String,
    avatar_url: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_contact_avatar(&state, email, avatar_url).await
}

#[tauri::command]
pub async fn db_get_all_accounts(state: State<'_, DbState>) -> Result<Vec<DbAccount>, String> {
    ratatoskr_core::db::queries_extra::db_get_all_accounts(&state).await
}

#[tauri::command]
pub async fn db_get_account(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbAccount>, String> {
    ratatoskr_core::db::queries_extra::db_get_account(&state, id).await
}

#[tauri::command]
pub async fn db_get_account_by_email(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<DbAccount>, String> {
    ratatoskr_core::db::queries_extra::db_get_account_by_email(&state, email).await
}

#[tauri::command]
pub async fn db_insert_account(
    state: State<'_, DbState>,
    id: String,
    email: String,
    display_name: Option<String>,
    avatar_url: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    token_expires_at: Option<i64>,
    provider: String,
    auth_method: String,
    imap_host: Option<String>,
    imap_port: Option<i64>,
    imap_security: Option<String>,
    smtp_host: Option<String>,
    smtp_port: Option<i64>,
    smtp_security: Option<String>,
    imap_password: Option<String>,
    oauth_provider: Option<String>,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    imap_username: Option<String>,
    accept_invalid_certs: Option<i64>,
    caldav_url: Option<String>,
    caldav_username: Option<String>,
    caldav_password: Option<String>,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: Option<String>,
    jmap_url: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_account(
        &state,
        id,
        email,
        display_name,
        avatar_url,
        access_token,
        refresh_token,
        token_expires_at,
        provider,
        auth_method,
        imap_host,
        imap_port,
        imap_security,
        smtp_host,
        smtp_port,
        smtp_security,
        imap_password,
        oauth_provider,
        oauth_client_id,
        oauth_client_secret,
        imap_username,
        accept_invalid_certs,
        caldav_url,
        caldav_username,
        caldav_password,
        caldav_principal_url,
        caldav_home_url,
        calendar_provider,
        jmap_url,
    )
    .await
}

#[tauri::command]
pub async fn db_update_account_tokens(
    state: State<'_, DbState>,
    id: String,
    access_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_account_tokens(
        &state,
        id,
        access_token,
        token_expires_at,
    )
    .await
}

#[tauri::command]
pub async fn db_update_account_all_tokens(
    state: State<'_, DbState>,
    id: String,
    access_token: String,
    refresh_token: String,
    token_expires_at: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_account_all_tokens(
        &state,
        id,
        access_token,
        refresh_token,
        token_expires_at,
    )
    .await
}

#[tauri::command]
pub async fn db_update_account_sync_state(
    state: State<'_, DbState>,
    id: String,
    history_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_account_sync_state(&state, id, history_id).await
}

#[tauri::command]
pub async fn db_clear_account_history_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_clear_account_history_id(&state, id).await
}

#[tauri::command]
pub async fn db_delete_account(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_account(&state, id).await
}

#[tauri::command]
pub async fn db_update_account_caldav(
    state: State<'_, DbState>,
    id: String,
    caldav_url: String,
    caldav_username: String,
    caldav_password: String,
    caldav_principal_url: Option<String>,
    caldav_home_url: Option<String>,
    calendar_provider: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_account_caldav(
        &state,
        id,
        caldav_url,
        caldav_username,
        caldav_password,
        caldav_principal_url,
        caldav_home_url,
        calendar_provider,
    )
    .await
}

#[tauri::command]
pub async fn db_upsert_thread(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    subject: Option<String>,
    snippet: Option<String>,
    last_message_at: Option<i64>,
    message_count: i64,
    is_read: bool,
    is_starred: bool,
    is_important: bool,
    has_attachments: bool,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_thread(
        &state,
        id,
        account_id,
        subject,
        snippet,
        last_message_at,
        message_count,
        is_read,
        is_starred,
        is_important,
        has_attachments,
    )
    .await
}

#[tauri::command]
pub async fn db_set_thread_labels(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_ids: Vec<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_thread_labels(
        &state, account_id, thread_id, label_ids,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_all_threads_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_all_threads_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_muted_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_muted_thread_ids(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_unread_inbox_count(state: State<'_, DbState>) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_get_unread_inbox_count(&state).await
}

#[tauri::command]
pub async fn db_get_messages_by_ids(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<super::types::DbMessage>, String> {
    ratatoskr_core::db::queries_extra::db_get_messages_by_ids(&state, account_id, message_ids).await
}

#[tauri::command]
pub async fn db_upsert_message(
    state: State<'_, DbState>,
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
    imap_folder: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_message(
        &state,
        id,
        account_id,
        thread_id,
        from_address,
        from_name,
        to_addresses,
        cc_addresses,
        bcc_addresses,
        reply_to,
        subject,
        snippet,
        date,
        is_read,
        is_starred,
        body_cached,
        raw_size,
        internal_date,
        list_unsubscribe,
        list_unsubscribe_post,
        auth_results,
        message_id_header,
        references_header,
        in_reply_to_header,
        imap_uid,
        imap_folder,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_message(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_message(&state, account_id, message_id).await
}

#[tauri::command]
pub async fn db_update_message_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
    thread_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_message_thread_ids(
        &state,
        account_id,
        message_ids,
        thread_id,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_all_messages_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_all_messages_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_recent_sent_messages(
    state: State<'_, DbState>,
    account_id: String,
    account_email: String,
    limit: Option<i64>,
) -> Result<Vec<super::types::DbMessage>, String> {
    ratatoskr_core::db::queries_extra::db_get_recent_sent_messages(
        &state,
        account_id,
        account_email,
        limit,
    )
    .await
}

#[tauri::command]
pub async fn db_get_tasks_for_account(
    state: State<'_, DbState>,
    account_id: Option<String>,
    include_completed: Option<bool>,
) -> Result<Vec<DbTask>, String> {
    ratatoskr_core::db::queries_extra::db_get_tasks_for_account(
        &state,
        account_id,
        include_completed,
    )
    .await
}

#[tauri::command]
pub async fn db_get_task_by_id(
    state: State<'_, DbState>,
    id: String,
) -> Result<Option<DbTask>, String> {
    ratatoskr_core::db::queries_extra::db_get_task_by_id(&state, id).await
}

#[tauri::command]
pub async fn db_get_tasks_for_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbTask>, String> {
    ratatoskr_core::db::queries_extra::db_get_tasks_for_thread(&state, account_id, thread_id).await
}

#[tauri::command]
pub async fn db_get_subtasks(
    state: State<'_, DbState>,
    parent_id: String,
) -> Result<Vec<DbTask>, String> {
    ratatoskr_core::db::queries_extra::db_get_subtasks(&state, parent_id).await
}

#[tauri::command]
pub async fn db_insert_task(
    state: State<'_, DbState>,
    id: String,
    account_id: Option<String>,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    parent_id: Option<String>,
    thread_id: Option<String>,
    thread_account_id: Option<String>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    tags_json: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_insert_task(
        &state,
        id,
        account_id,
        title,
        description,
        priority,
        due_date,
        parent_id,
        thread_id,
        thread_account_id,
        sort_order,
        recurrence_rule,
        tags_json,
    )
    .await
}

#[tauri::command]
pub async fn db_update_task(
    state: State<'_, DbState>,
    id: String,
    title: Option<String>,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    next_recurrence_at: Option<i64>,
    tags_json: Option<String>,
    // Sentinel flags to distinguish "set to null" from "not provided"
    clear_description: Option<bool>,
    clear_due_date: Option<bool>,
    clear_recurrence_rule: Option<bool>,
    clear_next_recurrence_at: Option<bool>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_task(
        &state,
        id,
        title,
        description,
        priority,
        due_date,
        sort_order,
        recurrence_rule,
        next_recurrence_at,
        tags_json, // Sentinel flags to distinguish "set to null" from "not provided"
        clear_description,
        clear_due_date,
        clear_recurrence_rule,
        clear_next_recurrence_at,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_task(&state, id).await
}

#[tauri::command]
pub async fn db_complete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_complete_task(&state, id).await
}

#[tauri::command]
pub async fn db_uncomplete_task(state: State<'_, DbState>, id: String) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_uncomplete_task(&state, id).await
}

#[tauri::command]
pub async fn db_reorder_tasks(
    state: State<'_, DbState>,
    task_ids: Vec<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_reorder_tasks(&state, task_ids).await
}

#[tauri::command]
pub async fn db_get_incomplete_task_count(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_get_incomplete_task_count(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_task_tags(
    state: State<'_, DbState>,
    account_id: Option<String>,
) -> Result<Vec<DbTaskTag>, String> {
    ratatoskr_core::db::queries_extra::db_get_task_tags(&state, account_id).await
}

#[tauri::command]
pub async fn db_upsert_task_tag(
    state: State<'_, DbState>,
    tag: String,
    account_id: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_task_tag(&state, tag, account_id, color).await
}

#[tauri::command]
pub async fn db_delete_task_tag(
    state: State<'_, DbState>,
    tag: String,
    account_id: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_task_tag(&state, tag, account_id).await
}

#[tauri::command]
pub async fn db_get_bundle_rule(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<Option<DbBundleRule>, String> {
    ratatoskr_core::db::queries_extra::db_get_bundle_rule(&state, account_id, category).await
}

#[tauri::command]
pub async fn db_set_bundle_rule(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    is_bundled: bool,
    delivery_enabled: bool,
    schedule: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_bundle_rule(
        &state,
        account_id,
        category,
        is_bundled,
        delivery_enabled,
        schedule,
    )
    .await
}

#[tauri::command]
pub async fn db_hold_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    category: String,
    held_until: Option<i64>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_hold_thread(
        &state, account_id, thread_id, category, held_until,
    )
    .await
}

#[tauri::command]
pub async fn db_is_thread_held(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    now: i64,
) -> Result<bool, String> {
    ratatoskr_core::db::queries_extra::db_is_thread_held(&state, account_id, thread_id, now).await
}

#[tauri::command]
pub async fn db_release_held_threads(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_release_held_threads(&state, account_id, category).await
}

#[tauri::command]
pub async fn db_update_last_delivered(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    now: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_last_delivered(&state, account_id, category, now)
        .await
}

#[tauri::command]
pub async fn db_get_bundle_summary(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
) -> Result<BundleSummarySingle, String> {
    ratatoskr_core::db::queries_extra::db_get_bundle_summary(&state, account_id, category).await
}

#[tauri::command]
pub async fn db_get_thread_category(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_thread_category(&state, account_id, thread_id).await
}

#[tauri::command]
pub async fn db_get_thread_category_with_manual(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<ThreadCategoryWithManual>, String> {
    ratatoskr_core::db::queries_extra::db_get_thread_category_with_manual(
        &state, account_id, thread_id,
    )
    .await
}

#[tauri::command]
pub async fn db_get_recent_rule_categorized_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_recent_rule_categorized_thread_ids(
        &state, account_id, limit,
    )
    .await
}

#[tauri::command]
pub async fn db_set_thread_categories_batch(
    state: State<'_, DbState>,
    account_id: String,
    categories: Vec<(String, String)>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_thread_categories_batch(
        &state, account_id, categories,
    )
    .await
}

#[tauri::command]
pub async fn db_get_uncategorized_inbox_thread_ids(
    state: State<'_, DbState>,
    account_id: String,
    limit: Option<i64>,
) -> Result<Vec<ThreadInfoRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_uncategorized_inbox_thread_ids(
        &state, account_id, limit,
    )
    .await
}

#[tauri::command]
pub async fn db_upsert_calendar(
    state: State<'_, DbState>,
    account_id: String,
    provider: String,
    remote_id: String,
    display_name: Option<String>,
    color: Option<String>,
    is_primary: bool,
) -> Result<String, String> {
    ratatoskr_core::db::queries_extra::db_upsert_calendar(
        &state,
        account_id,
        provider,
        remote_id,
        display_name,
        color,
        is_primary,
    )
    .await
}

#[tauri::command]
pub async fn db_get_calendars_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    ratatoskr_core::db::queries_extra::db_get_calendars_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_visible_calendars(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    ratatoskr_core::db::queries_extra::db_get_visible_calendars(&state, account_id).await
}

#[tauri::command]
pub async fn db_set_calendar_visibility(
    state: State<'_, DbState>,
    calendar_id: String,
    visible: bool,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_set_calendar_visibility(&state, calendar_id, visible)
        .await
}

#[tauri::command]
pub async fn db_update_calendar_sync_token(
    state: State<'_, DbState>,
    calendar_id: String,
    sync_token: Option<String>,
    ctag: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_calendar_sync_token(
        &state,
        calendar_id,
        sync_token,
        ctag,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_calendars_for_account(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_calendars_for_account(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_calendar_by_id(
    state: State<'_, DbState>,
    calendar_id: String,
) -> Result<Option<DbCalendar>, String> {
    ratatoskr_core::db::queries_extra::db_get_calendar_by_id(&state, calendar_id).await
}

#[tauri::command]
pub async fn db_upsert_calendar_event(
    state: State<'_, DbState>,
    account_id: String,
    google_event_id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
    status: String,
    organizer_email: Option<String>,
    attendees_json: Option<String>,
    html_link: Option<String>,
    calendar_id: Option<String>,
    remote_event_id: Option<String>,
    etag: Option<String>,
    ical_data: Option<String>,
    uid: Option<String>,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_upsert_calendar_event(
        &state,
        account_id,
        google_event_id,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        attendees_json,
        html_link,
        calendar_id,
        remote_event_id,
        etag,
        ical_data,
        uid,
    )
    .await
}

#[tauri::command]
pub async fn db_get_calendar_events_in_range(
    state: State<'_, DbState>,
    account_id: String,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    ratatoskr_core::db::queries_extra::db_get_calendar_events_in_range(
        &state, account_id, start_time, end_time,
    )
    .await
}

#[tauri::command]
pub async fn db_get_calendar_events_in_range_multi(
    state: State<'_, DbState>,
    account_id: String,
    calendar_ids: Vec<String>,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    ratatoskr_core::db::queries_extra::db_get_calendar_events_in_range_multi(
        &state,
        account_id,
        calendar_ids,
        start_time,
        end_time,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_events_for_calendar(
    state: State<'_, DbState>,
    calendar_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_events_for_calendar(&state, calendar_id).await
}

#[tauri::command]
pub async fn db_get_event_by_remote_id(
    state: State<'_, DbState>,
    calendar_id: String,
    remote_event_id: String,
) -> Result<Option<DbCalendarEvent>, String> {
    ratatoskr_core::db::queries_extra::db_get_event_by_remote_id(
        &state,
        calendar_id,
        remote_event_id,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_event_by_remote_id(
    state: State<'_, DbState>,
    calendar_id: String,
    remote_event_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_event_by_remote_id(
        &state,
        calendar_id,
        remote_event_id,
    )
    .await
}

#[tauri::command]
pub async fn db_delete_calendar_event(
    state: State<'_, DbState>,
    event_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_delete_calendar_event(&state, event_id).await
}

#[tauri::command]
pub async fn db_get_snoozed_threads_due(
    state: State<'_, DbState>,
    now: i64,
) -> Result<Vec<SnoozedThread>, String> {
    ratatoskr_core::db::queries_extra::db_get_snoozed_threads_due(&state, now).await
}

#[tauri::command]
pub async fn db_record_unsubscribe_action(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    thread_id: String,
    from_address: String,
    from_name: Option<String>,
    method: String,
    unsubscribe_url: String,
    status: String,
    now: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_record_unsubscribe_action(
        &state,
        id,
        account_id,
        thread_id,
        from_address,
        from_name,
        method,
        unsubscribe_url,
        status,
        now,
    )
    .await
}

#[tauri::command]
pub async fn db_get_subscriptions(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<SubscriptionEntry>, String> {
    ratatoskr_core::db::queries_extra::db_get_subscriptions(&state, account_id).await
}

#[tauri::command]
pub async fn db_get_unsubscribe_status(
    state: State<'_, DbState>,
    account_id: String,
    from_address: String,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_get_unsubscribe_status(&state, account_id, from_address)
        .await
}

#[tauri::command]
pub async fn db_get_imap_uids_for_messages(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
) -> Result<Vec<ImapMessageRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_imap_uids_for_messages(
        &state,
        account_id,
        message_ids,
    )
    .await
}

#[tauri::command]
pub async fn db_find_special_folder(
    state: State<'_, DbState>,
    account_id: String,
    special_use: String,
    fallback_label_id: Option<String>,
) -> Result<Option<String>, String> {
    ratatoskr_core::db::queries_extra::db_find_special_folder(
        &state,
        account_id,
        special_use,
        fallback_label_id,
    )
    .await
}

#[tauri::command]
pub async fn db_update_message_imap_folder(
    state: State<'_, DbState>,
    account_id: String,
    message_ids: Vec<String>,
    new_folder: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_message_imap_folder(
        &state,
        account_id,
        message_ids,
        new_folder,
    )
    .await
}

#[tauri::command]
pub async fn db_update_attachment_cached(
    state: State<'_, DbState>,
    attachment_id: String,
    local_path: String,
    cache_size: i64,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_attachment_cached(
        &state,
        attachment_id,
        local_path,
        cache_size,
    )
    .await
}

#[tauri::command]
pub async fn db_get_attachment_cache_size(state: State<'_, DbState>) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_get_attachment_cache_size(&state).await
}

#[tauri::command]
pub async fn db_get_oldest_cached_attachments(
    state: State<'_, DbState>,
    limit: i64,
) -> Result<Vec<CachedAttachmentRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_oldest_cached_attachments(&state, limit).await
}

#[tauri::command]
pub async fn db_clear_attachment_cache_entry(
    state: State<'_, DbState>,
    attachment_id: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_clear_attachment_cache_entry(&state, attachment_id).await
}

#[tauri::command]
pub async fn db_clear_all_attachment_cache(state: State<'_, DbState>) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_clear_all_attachment_cache(&state).await
}

#[tauri::command]
pub async fn db_count_cached_by_hash(
    state: State<'_, DbState>,
    content_hash: String,
) -> Result<i64, String> {
    ratatoskr_core::db::queries_extra::db_count_cached_by_hash(&state, content_hash).await
}

#[tauri::command]
pub async fn db_get_inbox_threads_for_backfill(
    state: State<'_, DbState>,
    account_id: String,
    batch_size: i64,
    offset: i64,
) -> Result<Vec<BackfillRow>, String> {
    ratatoskr_core::db::queries_extra::db_get_inbox_threads_for_backfill(
        &state, account_id, batch_size, offset,
    )
    .await
}

#[tauri::command]
pub async fn db_update_scheduled_email_attachments(
    state: State<'_, DbState>,
    account_id: String,
    attachment_data: String,
) -> Result<(), String> {
    ratatoskr_core::db::queries_extra::db_update_scheduled_email_attachments(
        &state,
        account_id,
        attachment_data,
    )
    .await
}

#[tauri::command]
pub async fn db_query_raw_select(
    state: State<'_, DbState>,
    sql: String,
    params: Vec<serde_json::Value>,
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>, String> {
    ratatoskr_core::db::queries_extra::db_query_raw_select(&state, sql, params).await
}
