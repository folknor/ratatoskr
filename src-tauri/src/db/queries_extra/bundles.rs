#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{
    BundleSummary, BundleSummarySingle, DbBundleRule, DbCategory, ThreadCategoryWithManual,
    ThreadInfoRow,
};

db_command! {
    fn db_set_thread_category(state, account_id: String, thread_id: String, category: String, is_manual: bool) -> ();
    fn db_get_bundle_rules(state, account_id: String) -> Vec<DbBundleRule>;
    fn db_get_bundle_summaries(state, account_id: String, categories: Vec<String>) -> Vec<BundleSummary>;
    fn db_get_held_thread_ids(state, account_id: String) -> Vec<String>;
    fn db_get_bundle_rule(state, account_id: String, category: String) -> Option<DbBundleRule>;
    fn db_set_bundle_rule(state, account_id: String, category: String, is_bundled: bool, delivery_enabled: bool, schedule: Option<String>) -> ();
    fn db_hold_thread(state, account_id: String, thread_id: String, category: String, held_until: Option<i64>) -> ();
    fn db_is_thread_held(state, account_id: String, thread_id: String, now: i64) -> bool;
    fn db_release_held_threads(state, account_id: String, category: String) -> i64;
    fn db_update_last_delivered(state, account_id: String, category: String, now: i64) -> ();
    fn db_get_bundle_summary(state, account_id: String, category: String) -> BundleSummarySingle;
    fn db_get_thread_category(state, account_id: String, thread_id: String) -> Option<String>;
    fn db_get_thread_category_with_manual(state, account_id: String, thread_id: String) -> Option<ThreadCategoryWithManual>;
    fn db_get_recent_rule_categorized_thread_ids(state, account_id: String, limit: Option<i64>) -> Vec<ThreadInfoRow>;
    fn db_set_thread_categories_batch(state, account_id: String, categories: Vec<(String, String)>) -> ();
    fn db_get_uncategorized_inbox_thread_ids(state, account_id: String, limit: Option<i64>) -> Vec<ThreadInfoRow>;
    fn db_get_categories(state, account_id: String) -> Vec<DbCategory>;
}
