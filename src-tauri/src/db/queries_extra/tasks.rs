#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{DbTask, DbTaskTag};

db_command! {
    fn db_get_tasks_for_account(state, account_id: Option<String>, include_completed: Option<bool>) -> Vec<DbTask>;
    fn db_get_task_by_id(state, id: String) -> Option<DbTask>;
    fn db_get_tasks_for_thread(state, account_id: String, thread_id: String) -> Vec<DbTask>;
    fn db_get_subtasks(state, parent_id: String) -> Vec<DbTask>;
    fn db_insert_task(
        state,
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
        tags_json: Option<String>
    ) -> ();
    fn db_update_task(
        state,
        id: String,
        title: Option<String>,
        description: Option<String>,
        priority: Option<String>,
        due_date: Option<i64>,
        sort_order: Option<i64>,
        recurrence_rule: Option<String>,
        next_recurrence_at: Option<i64>,
        tags_json: Option<String>,
        clear_description: Option<bool>,
        clear_due_date: Option<bool>,
        clear_recurrence_rule: Option<bool>,
        clear_next_recurrence_at: Option<bool>
    ) -> ();
    fn db_delete_task(state, id: String) -> ();
    fn db_complete_task(state, id: String) -> ();
    fn db_uncomplete_task(state, id: String) -> ();
    fn db_reorder_tasks(state, task_ids: Vec<String>) -> ();
    fn db_get_incomplete_task_count(state, account_id: Option<String>) -> i64;
    fn db_get_task_tags(state, account_id: Option<String>) -> Vec<DbTaskTag>;
    fn db_upsert_task_tag(state, tag: String, account_id: Option<String>, color: Option<String>) -> ();
    fn db_delete_task_tag(state, tag: String, account_id: Option<String>) -> ();
}
