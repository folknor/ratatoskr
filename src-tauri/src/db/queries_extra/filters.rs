#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{
    DbFilterRule, DbFollowUpReminder, DbQuickStep, DbSmartFolder, DbSmartLabelRule, SortOrderItem,
    TriggeredFollowUp,
};

db_command! {
    fn db_get_filters_for_account(state, account_id: String) -> Vec<DbFilterRule>;
    fn db_insert_filter(state, id: String, account_id: String, name: String, criteria_json: String, actions_json: String, is_enabled: Option<bool>) -> ();
    fn db_update_filter(state, id: String, name: Option<String>, criteria_json: Option<String>, actions_json: Option<String>, is_enabled: Option<bool>) -> ();
    fn db_delete_filter(state, id: String) -> ();
}

// ── Smart Folders ───────────────────────────────────────────

db_command! {
    fn db_get_smart_folders(state, account_id: Option<String>) -> Vec<DbSmartFolder>;
    fn db_get_smart_folder_by_id(state, id: String) -> Option<DbSmartFolder>;
    fn db_insert_smart_folder(state, id: String, name: String, query: String, account_id: Option<String>, icon: Option<String>, color: Option<String>) -> ();
    fn db_update_smart_folder(state, id: String, name: Option<String>, query: Option<String>, icon: Option<String>, color: Option<String>) -> ();
    fn db_delete_smart_folder(state, id: String) -> ();
    fn db_update_smart_folder_sort_order(state, orders: Vec<SortOrderItem>) -> ();
}

// ── Smart Label Rules ───────────────────────────────────────

db_command! {
    fn db_get_smart_label_rules_for_account(state, account_id: String) -> Vec<DbSmartLabelRule>;
    fn db_insert_smart_label_rule(state, id: String, account_id: String, label_id: String, ai_description: String, criteria_json: Option<String>, is_enabled: Option<bool>) -> ();
    fn db_update_smart_label_rule(state, id: String, label_id: Option<String>, ai_description: Option<String>, criteria_json: Option<String>, is_enabled: Option<bool>) -> ();
    fn db_delete_smart_label_rule(state, id: String) -> ();
}

// ── Follow-Up Reminders ─────────────────────────────────────

db_command! {
    fn db_insert_follow_up_reminder(state, id: String, account_id: String, thread_id: String, message_id: String, remind_at: i64) -> ();
    fn db_get_follow_up_for_thread(state, account_id: String, thread_id: String) -> Option<DbFollowUpReminder>;
    fn db_cancel_follow_up_for_thread(state, account_id: String, thread_id: String) -> ();
    fn db_get_active_follow_up_thread_ids(state, account_id: String, thread_ids: Vec<String>) -> Vec<String>;
    fn db_check_follow_up_reminders(state) -> Vec<TriggeredFollowUp>;
}

// ── Quick Steps ─────────────────────────────────────────────

db_command! {
    fn db_get_quick_steps_for_account(state, account_id: String) -> Vec<DbQuickStep>;
    fn db_get_enabled_quick_steps_for_account(state, account_id: String) -> Vec<DbQuickStep>;
    fn db_insert_quick_step(state, step: DbQuickStep) -> ();
    fn db_update_quick_step(state, step: DbQuickStep) -> ();
    fn db_delete_quick_step(state, id: String) -> ();
}
