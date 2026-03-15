#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{BackfillRow, CachedAttachmentRow, UncachedAttachment};

db_command! {
    fn db_attachment_cache_total_size(state) -> i64;
    fn db_uncached_recent_attachments(state, max_size: i64, cutoff_epoch: i64, limit: i64) -> Vec<UncachedAttachment>;
    fn db_get_ai_cache(state, account_id: String, thread_id: String, cache_type: String) -> Option<String>;
    fn db_set_ai_cache(state, account_id: String, thread_id: String, cache_type: String, content: String) -> ();
    fn db_delete_ai_cache(state, account_id: String, thread_id: String, cache_type: String) -> ();
    fn db_get_cached_scan_result(state, account_id: String, message_id: String) -> Option<String>;
    fn db_cache_scan_result(state, account_id: String, message_id: String, result_json: String) -> ();
    fn db_delete_scan_results(state, account_id: String) -> ();
    fn db_update_attachment_cached(state, attachment_id: String, local_path: String, cache_size: i64) -> ();
    fn db_get_attachment_cache_size(state) -> i64;
    fn db_get_oldest_cached_attachments(state, limit: i64) -> Vec<CachedAttachmentRow>;
    fn db_clear_attachment_cache_entry(state, attachment_id: String) -> ();
    fn db_clear_all_attachment_cache(state) -> ();
    fn db_count_cached_by_hash(state, content_hash: String) -> i64;
    fn db_get_inbox_threads_for_backfill(state, account_id: String, batch_size: i64, offset: i64) -> Vec<BackfillRow>;
    fn db_query_raw_select(state, sql: String, params: Vec<serde_json::Value>) -> Vec<serde_json::Map<String, serde_json::Value>>;
}
