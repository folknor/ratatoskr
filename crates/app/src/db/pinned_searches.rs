use rtsk::db::pinned_searches::{
    DbPinnedSearch, db_create_or_update_pinned_search, db_delete_all_pinned_searches,
    db_delete_pinned_search, db_expire_stale_pinned_searches, db_get_pinned_search_thread_ids,
    db_get_recent_search_queries, db_get_threads_by_ids, db_list_pinned_searches,
    db_update_pinned_search,
};
use rtsk::db::queries_extra::db_insert_smart_folder;
use rtsk::db::types::DbThread;

use super::connection::Db;
use super::types::Thread;

/// A pinned search with its stored thread snapshot.
#[derive(Debug, Clone)]
pub struct PinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub scope_account_id: Option<String>,
    #[allow(dead_code)]
    pub thread_ids: Option<Vec<(String, String)>>,
}

fn db_pinned_search_to_app(ps: DbPinnedSearch) -> PinnedSearch {
    PinnedSearch {
        id: ps.id,
        query: ps.query,
        created_at: ps.created_at,
        updated_at: ps.updated_at,
        scope_account_id: ps.scope_account_id,
        thread_ids: ps.thread_ids,
    }
}

fn db_thread_to_app_thread(t: DbThread) -> Thread {
    Thread {
        id: t.id,
        account_id: t.account_id,
        subject: t.subject,
        snippet: t.snippet,
        last_message_at: t.last_message_at,
        message_count: t.message_count,
        is_read: t.is_read,
        is_starred: t.is_starred,
        is_pinned: t.is_pinned,
        is_muted: t.is_muted,
        has_attachments: t.has_attachments,
        from_name: t.from_name,
        from_address: t.from_address,
        is_local_draft: false,
    }
}

impl Db {
    pub async fn create_or_update_pinned_search(
        &self,
        query: String,
        thread_ids: Vec<(String, String)>,
        scope_account_id: Option<String>,
    ) -> Result<i64, String> {
        let db = self.write_db_state();
        db_create_or_update_pinned_search(&db, query, thread_ids, scope_account_id).await
    }

    pub async fn update_pinned_search(
        &self,
        id: i64,
        query: String,
        thread_ids: Vec<(String, String)>,
        scope_account_id: Option<String>,
    ) -> Result<(), String> {
        let db = self.write_db_state();
        db_update_pinned_search(&db, id, query, thread_ids, scope_account_id).await
    }

    pub async fn delete_pinned_search(&self, id: i64) -> Result<(), String> {
        let db = self.write_db_state();
        db_delete_pinned_search(&db, id).await
    }

    pub async fn list_pinned_searches(&self) -> Result<Vec<PinnedSearch>, String> {
        let db = self.read_db_state();
        Ok(db_list_pinned_searches(&db)
            .await?
            .into_iter()
            .map(db_pinned_search_to_app)
            .collect())
    }

    pub async fn get_pinned_search_thread_ids(
        &self,
        pinned_search_id: i64,
    ) -> Result<Vec<(String, String)>, String> {
        let db = self.read_db_state();
        db_get_pinned_search_thread_ids(&db, pinned_search_id).await
    }

    pub async fn get_threads_by_ids(
        &self,
        ids: Vec<(String, String)>,
    ) -> Result<Vec<Thread>, String> {
        let db = self.read_db_state();
        Ok(db_get_threads_by_ids(&db, ids)
            .await?
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect())
    }

    pub async fn get_recent_search_queries(&self, limit: usize) -> Result<Vec<String>, String> {
        let db = self.read_db_state();
        db_get_recent_search_queries(&db, limit).await
    }

    pub async fn delete_all_pinned_searches(&self) -> Result<u64, String> {
        let db = self.write_db_state();
        db_delete_all_pinned_searches(&db).await
    }

    pub async fn create_smart_folder(&self, name: String, query: String) -> Result<i64, String> {
        let db = self.write_db_state();
        let id = uuid::Uuid::new_v4().to_string();
        db_insert_smart_folder(&db, id, name, query, None, None, None).await?;
        Ok(0)
    }

    pub async fn expire_stale_pinned_searches(&self, max_age_secs: i64) -> Result<u64, String> {
        let db = self.write_db_state();
        db_expire_stale_pinned_searches(&db, max_age_secs).await
    }
}
