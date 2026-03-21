use std::sync::Arc;

use iced::Task;

use crate::db::{self, Thread};
use crate::ui::sidebar::truncate_query;
use crate::ui::thread_list::ThreadListMode;
use crate::{App, Message};

// ── Search handling ────────────────────────────────────

impl App {
    pub(crate) fn handle_search_query_changed(&mut self, query: String) -> Task<Message> {
        log::debug!("Search query changed: {query:?}");
        self.search_query = query;
        self.thread_list.search_query.clone_from(&self.search_query);
        if self.search_query.trim().is_empty() {
            self.search_debounce_deadline = None;
            if self.thread_list.mode == ThreadListMode::Search {
                self.clear_pinned_search_context();
                self.nav_generation += 1;
                self.search_generation += 1;
                return self.restore_folder_view();
            }
        } else {
            self.search_debounce_deadline = Some(
                iced::time::Instant::now()
                    + std::time::Duration::from_millis(150),
            );
        }
        Task::none()
    }

    pub(crate) fn handle_search_execute(&mut self) -> Task<Message> {
        self.search_debounce_deadline = None;
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            return self.restore_folder_view();
        }
        log::info!("Search executing: {query:?}");

        // Store pre-search threads on first search from folder mode
        if self.thread_list.mode == ThreadListMode::Folder {
            self.pre_search_threads = Some(self.thread_list.threads.clone());
        }

        self.search_generation += 1;
        let generation = self.search_generation;
        let db = Arc::clone(&self.db);

        Task::perform(
            async move {
                let result = execute_search(db, query).await;
                (generation, result)
            },
            |(g, result)| Message::SearchResultsLoaded(g, result),
        )
    }

    pub(crate) fn handle_search_results(
        &mut self,
        result: Result<Vec<Thread>, String>,
    ) -> Task<Message> {
        match result {
            Ok(threads) => {
                self.thread_list.mode = ThreadListMode::Search;
                log::debug!("Search results: {} threads", threads.len());
                self.status = format!("{} results", threads.len());

                let thread_ids: Vec<(String, String)> = threads
                    .iter()
                    .map(|t| (t.id.clone(), t.account_id.clone()))
                    .collect();
                let query = self.search_query.clone();

                self.thread_list.set_threads(threads);
                self.thread_list.selected_thread = None;

                // Create or update pinned search
                if !query.trim().is_empty() {
                    let db = Arc::clone(&self.db);
                    if let Some(editing_id) = self.editing_pinned_search {
                        return Task::perform(
                            async move {
                                db.update_pinned_search(editing_id, query, thread_ids)
                                    .await
                                    .map(|()| editing_id)
                            },
                            Message::PinnedSearchSaved,
                        );
                    }
                    return Task::perform(
                        async move {
                            db.create_or_update_pinned_search(query, thread_ids).await
                        },
                        Message::PinnedSearchSaved,
                    );
                }
                Task::none()
            }
            Err(e) => {
                log::error!("Search failed: {e}");
                self.status = format!("Search error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_search_clear(&mut self) -> Task<Message> {
        self.search_query.clear();
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        self.search_generation += 1;
        self.restore_folder_view()
    }

    pub(crate) fn handle_focus_search_bar(&self) -> Task<Message> {
        iced::widget::operation::focus::<Message>("search-bar".to_string())
    }
}

// ── Pinned search handling ─────────────────────────────

impl App {
    pub(crate) fn handle_pinned_searches_loaded(
        &mut self,
        result: Result<Vec<db::PinnedSearch>, String>,
    ) -> Task<Message> {
        match result {
            Ok(searches) => {
                self.pinned_searches = searches;
                self.sidebar.pinned_searches.clone_from(&self.pinned_searches);

                if !self.expiry_ran {
                    self.expiry_ran = true;
                    let db = Arc::clone(&self.db);
                    return Task::perform(
                        async move { db.expire_stale_pinned_searches(1_209_600).await },
                        Message::PinnedSearchesExpired,
                    );
                }
                Task::none()
            }
            Err(e) => {
                log::error!("Failed to load pinned searches: {e}");
                self.status = format!("Pinned searches error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_select_pinned_search(&mut self, id: i64) -> Task<Message> {
        log::debug!("Pinned search selected: id={id}");
        // Save pre-search threads on first activation from folder mode
        if self.active_pinned_search.is_none()
            && self.thread_list.mode == ThreadListMode::Folder
        {
            self.pre_search_threads = Some(self.thread_list.threads.clone());
        }

        self.active_pinned_search = Some(id);
        self.sidebar.active_pinned_search = Some(id);
        self.editing_pinned_search = Some(id);
        self.sidebar.selected_label = None;

        self.nav_generation += 1;
        self.thread_generation += 1;
        self.thread_list.selected_thread = None;

        // Update thread list context
        if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == id) {
            let label = truncate_query(&ps.query, 30);
            self.thread_list
                .set_context(format!("Search: {label}"), "All Accounts".to_string());
        }

        let db = Arc::clone(&self.db);
        let load_gen = self.nav_generation;
        Task::perform(
            async move {
                let ids = db.get_pinned_search_thread_ids(id).await;
                (load_gen, id, ids)
            },
            |(g, id, result)| Message::PinnedSearchThreadIdsLoaded(g, id, result),
        )
    }

    pub(crate) fn handle_pinned_search_thread_ids_loaded(
        &mut self,
        ps_id: i64,
        ids: Result<Vec<(String, String)>, String>,
    ) -> Task<Message> {
        match ids {
            Ok(ids) => {
                if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == ps_id) {
                    self.search_query.clone_from(&ps.query);
                    self.thread_list.search_query.clone_from(&ps.query);
                }

                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation;
                Task::perform(
                    async move {
                        let result = db.get_threads_by_ids(ids).await;
                        (load_gen, result)
                    },
                    |(g, result)| Message::PinnedSearchThreadsLoaded(g, result),
                )
            }
            Err(e) => {
                log::error!("Failed to load pinned search thread IDs: {e}");
                self.status = format!("Error loading pinned search: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_pinned_search_threads_loaded(
        &mut self,
        result: Result<Vec<Thread>, String>,
    ) -> Task<Message> {
        match result {
            Ok(threads) => {
                self.thread_list.mode = ThreadListMode::Search;
                log::debug!("Pinned search threads loaded: {} threads", threads.len());
                self.status = format!("{} threads (pinned search)", threads.len());
                self.thread_list.set_threads(threads);
                self.thread_list.selected_thread = None;
                Task::none()
            }
            Err(e) => {
                log::error!("Failed to load pinned search threads: {e}");
                self.status = format!("Threads error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_dismiss_pinned_search(&self, id: i64) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                let result = db.delete_pinned_search(id).await;
                (id, result)
            },
            |(id, result)| Message::PinnedSearchDismissed(id, result),
        )
    }

    pub(crate) fn handle_pinned_search_dismissed(
        &mut self,
        id: i64,
        result: Result<(), String>,
    ) -> Task<Message> {
        match result {
            Ok(()) => {
                log::info!("Pinned search deleted: id={id}");
                self.pinned_searches.retain(|ps| ps.id != id);
                self.sidebar.pinned_searches.retain(|ps| ps.id != id);
                if self.active_pinned_search == Some(id) {
                    self.active_pinned_search = None;
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    return self.restore_folder_view();
                }
                Task::none()
            }
            Err(e) => {
                log::error!("Failed to dismiss pinned search: {e}");
                self.status = format!("Dismiss error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_pinned_search_saved(
        &mut self,
        result: Result<i64, String>,
    ) -> Task<Message> {
        match result {
            Ok(id) => {
                log::info!("Pinned search saved: id={id}");
                self.active_pinned_search = Some(id);
                self.sidebar.active_pinned_search = Some(id);
                self.editing_pinned_search = Some(id);

                let db = Arc::clone(&self.db);
                Task::perform(
                    async move { db.list_pinned_searches().await },
                    Message::PinnedSearchesLoaded,
                )
            }
            Err(e) => {
                log::error!("Failed to save pinned search: {e}");
                self.status = format!("Save pinned search error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_pinned_searches_expired(
        &mut self,
        result: Result<u64, String>,
    ) -> Task<Message> {
        match result {
            Ok(count) => {
                if count > 0 {
                    let db = Arc::clone(&self.db);
                    Task::perform(
                        async move { db.list_pinned_searches().await },
                        Message::PinnedSearchesLoaded,
                    )
                } else {
                    Task::none()
                }
            }
            Err(e) => {
                log::warn!("Pinned search expiry failed: {e}");
                self.status = format!("Expiry warning: {e}");
                Task::none()
            }
        }
    }

    /// Clear pinned search context on navigate-away.
    pub(crate) fn clear_pinned_search_context(&mut self) {
        self.active_pinned_search = None;
        self.sidebar.active_pinned_search = None;
        self.editing_pinned_search = None;
    }

    /// Clear all search-related state without restoring pre-search threads.
    pub(crate) fn clear_search_state(&mut self) {
        self.search_query.clear();
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        self.search_generation += 1;
        self.thread_list.mode = ThreadListMode::Folder;
        self.pre_search_threads = None;
    }

    /// Restore the thread list to folder view after clearing search.
    pub(crate) fn restore_folder_view(&mut self) -> Task<Message> {
        self.thread_list.mode = ThreadListMode::Folder;
        self.search_query.clear();
        self.thread_list.search_query.clear();
        self.thread_list.selected_thread = None;
        self.reading_pane.thread_messages.clear();
        self.reading_pane.thread_attachments.clear();
        self.reading_pane.message_expanded.clear();
        if let Some(threads) = self.pre_search_threads.take() {
            self.status = format!("{} threads", threads.len());
            self.thread_list.set_threads(threads);
        }
        Task::none()
    }
}

/// Execute search off the main thread via spawn_blocking.
///
/// Tries the unified search pipeline (Tantivy + SQL) first. Falls back
/// to SQL-only via the smart folder parser if no Tantivy index exists,
/// and to a simple LIKE search for pure free-text without operators.
pub(crate) async fn execute_search(
    db: Arc<db::Db>,
    query: String,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let data_dir = crate::APP_DATA_DIR.get().ok_or("APP_DATA_DIR not set")?;
        match ratatoskr_core::search::SearchState::init(data_dir) {
            Ok(search_state) => {
                let results = ratatoskr_core::search_pipeline::search(
                    &query, &search_state, conn,
                )?;
                Ok(results.into_iter().map(unified_result_to_thread).collect())
            }
            Err(_) => {
                log::warn!("Tantivy index not available, falling back to SQL-only search");
                // Tantivy index not available — fall back to SQL-only
                execute_search_sql_fallback(conn, &query)
            }
        }
    })
    .await
}

/// SQL-only fallback search using the smart folder parser and SQL builder.
fn execute_search_sql_fallback(
    conn: &rusqlite::Connection,
    query: &str,
) -> Result<Vec<Thread>, String> {
    let parsed = ratatoskr_core::smart_folder::parse_query(query);
    let scope = ratatoskr_core::db::types::AccountScope::All;

    if parsed.has_any_operator() || parsed.free_text.is_empty() {
        let db_threads = ratatoskr_core::smart_folder::query_threads(
            conn, &parsed, &scope, Some(200), Some(0),
        )?;
        Ok(db_threads.into_iter().map(crate::db_thread_to_app_thread).collect())
    } else {
        // Free text only, no Tantivy — do a simple LIKE search
        let pattern = format!("%{}%", parsed.free_text);
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.account_id, t.subject, t.snippet,
                        t.last_message_at, t.message_count,
                        t.is_read, t.is_starred, t.has_attachments,
                        t.from_name, t.from_address
                 FROM threads t
                 WHERE t.subject LIKE ?1 OR t.snippet LIKE ?1
                 ORDER BY t.last_message_at DESC
                 LIMIT 200",
            )
            .map_err(|e| format!("prepare search: {e}"))?;
        let rows = stmt
            .query_map([&pattern], |row| {
                Ok(Thread {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    subject: row.get(2)?,
                    snippet: row.get(3)?,
                    last_message_at: row.get::<_, Option<String>>(4)?
                        .and_then(|s| s.parse().ok()),
                    message_count: row.get(5)?,
                    is_read: row.get(6)?,
                    is_starred: row.get(7)?,
                    has_attachments: row.get(8)?,
                    from_name: row.get(9)?,
                    from_address: row.get(10)?,
                })
            })
            .map_err(|e| format!("search query: {e}"))?;
        let mut threads = Vec::new();
        for row in rows {
            threads.push(row.map_err(|e| format!("search row: {e}"))?);
        }
        Ok(threads)
    }
}

/// Convert a `UnifiedSearchResult` from the search pipeline to an app `Thread`.
fn unified_result_to_thread(
    r: ratatoskr_core::search_pipeline::UnifiedSearchResult,
) -> Thread {
    Thread {
        id: r.thread_id,
        account_id: r.account_id,
        subject: r.subject,
        snippet: r.snippet,
        last_message_at: r.date,
        message_count: r.message_count.unwrap_or(1),
        is_read: r.is_read,
        is_starred: r.is_starred,
        has_attachments: false,
        from_name: r.from_name,
        from_address: r.from_address,
    }
}
