use std::sync::Arc;

use iced::Task;
use ratatoskr_smart_folder::analyze_cursor_context;

use crate::db::{self, Thread};
use crate::ui::sidebar::truncate_query;
use crate::ui::thread_list::{
    ThreadListMessage, ThreadListMode, TypeaheadItem,
};
use crate::{App, Message};

// ── Search handling ────────────────────────────────────

impl App {
    pub(crate) fn handle_search_query_changed(&mut self, query: String) -> Task<Message> {
        self.search_query = query;
        self.thread_list.search_query.clone_from(&self.search_query);
        if self.search_query.trim().is_empty() {
            self.search_debounce_deadline = None;
            self.thread_list.typeahead.visible = false;
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

            // Check for dynamic typeahead operators and trigger DB queries.
            if let Some(typeahead_task) = self.maybe_trigger_typeahead_query() {
                return typeahead_task;
            }
        }
        Task::none()
    }

    pub(crate) fn handle_search_execute(&mut self) -> Task<Message> {
        self.search_debounce_deadline = None;
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            return self.restore_folder_view();
        }

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

    /// Analyze the current query for dynamic typeahead operators and
    /// dispatch a DB query if needed.
    fn maybe_trigger_typeahead_query(&mut self) -> Option<Task<Message>> {
        let cursor_pos = self.search_query.len();
        let ctx = analyze_cursor_context(&self.search_query, cursor_pos);

        let ratatoskr_smart_folder::CursorContext::InsideOperator {
            ref operator,
            ref partial_value,
            ..
        } = ctx
        else {
            return None;
        };

        match operator.as_str() {
            "from" | "to" => {
                let db = Arc::clone(&self.db);
                let partial = partial_value.clone();
                Some(Task::perform(
                    async move {
                        db.search_autocomplete(partial, 10).await
                    },
                    |result| {
                        let items = match result {
                            Ok(contacts) => contacts
                                .into_iter()
                                .map(|c| TypeaheadItem {
                                    label: c
                                        .display_name
                                        .clone()
                                        .unwrap_or_else(|| c.email.clone()),
                                    detail: Some(c.email.clone()),
                                    insert_value: c.email,
                                })
                                .collect(),
                            Err(_) => Vec::new(),
                        };
                        Message::ThreadList(
                            ThreadListMessage::TypeaheadItemsLoaded(items),
                        )
                    },
                ))
            }
            "account" => {
                let db = Arc::clone(&self.db);
                let partial = partial_value.to_ascii_lowercase();
                Some(Task::perform(
                    async move { db.get_accounts().await },
                    move |result| {
                        let items = match result {
                            Ok(accounts) => accounts
                                .into_iter()
                                .filter(|a| {
                                    partial.is_empty()
                                        || a.email
                                            .to_ascii_lowercase()
                                            .contains(&partial)
                                        || a.display_name
                                            .as_ref()
                                            .is_some_and(|n| {
                                                n.to_ascii_lowercase()
                                                    .contains(&partial)
                                            })
                                        || a.account_name
                                            .as_ref()
                                            .is_some_and(|n| {
                                                n.to_ascii_lowercase()
                                                    .contains(&partial)
                                            })
                                })
                                .map(|a| {
                                    let label = a
                                        .account_name
                                        .or(a.display_name)
                                        .unwrap_or_else(|| a.email.clone());
                                    TypeaheadItem {
                                        label,
                                        detail: Some(a.email.clone()),
                                        insert_value: a.email,
                                    }
                                })
                                .collect(),
                            Err(_) => Vec::new(),
                        };
                        Message::ThreadList(
                            ThreadListMessage::TypeaheadItemsLoaded(items),
                        )
                    },
                ))
            }
            "label" | "folder" => {
                let db = Arc::clone(&self.db);
                let partial = partial_value.to_ascii_lowercase();
                // Get labels from all accounts (since we don't have easy
                // per-account scoping in the search bar context).
                Some(Task::perform(
                    async move { db.search_labels_for_typeahead(partial).await },
                    |result| {
                        let items = match result {
                            Ok(labels) => labels,
                            Err(_) => Vec::new(),
                        };
                        Message::ThreadList(
                            ThreadListMessage::TypeaheadItemsLoaded(items),
                        )
                    },
                ))
            }
            _ => None,
        }
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
                self.status = format!("Pinned searches error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_select_pinned_search(&mut self, id: i64) -> Task<Message> {
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

        // Use cached thread IDs if available
        let cached_ids = self
            .pinned_searches
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.thread_ids.clone());

        if let Some(ids) = cached_ids {
            let load_gen = self.nav_generation;
            return self.handle_pinned_search_thread_ids_loaded_inner(
                load_gen, id, Ok(ids),
            );
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
        let load_gen = self.nav_generation;
        self.handle_pinned_search_thread_ids_loaded_inner(load_gen, ps_id, ids)
    }

    fn handle_pinned_search_thread_ids_loaded_inner(
        &mut self,
        load_gen: u64,
        ps_id: i64,
        ids: Result<Vec<(String, String)>, String>,
    ) -> Task<Message> {
        match ids {
            Ok(ids) => {
                // Cache the thread IDs on the pinned search
                for ps in &mut self.pinned_searches {
                    if ps.id == ps_id {
                        ps.thread_ids = Some(ids.clone());
                        break;
                    }
                }
                // Also update the sidebar copy
                for ps in &mut self.sidebar.pinned_searches {
                    if ps.id == ps_id {
                        ps.thread_ids = Some(ids.clone());
                        break;
                    }
                }

                if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == ps_id) {
                    self.search_query.clone_from(&ps.query);
                    self.thread_list.search_query.clone_from(&ps.query);
                }

                let db = Arc::clone(&self.db);
                Task::perform(
                    async move {
                        let result = db.get_threads_by_ids(ids).await;
                        (load_gen, result)
                    },
                    |(g, result)| Message::PinnedSearchThreadsLoaded(g, result),
                )
            }
            Err(e) => {
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
                self.status = format!("{} threads (pinned search)", threads.len());
                self.thread_list.set_threads(threads);
                self.thread_list.selected_thread = None;
                Task::none()
            }
            Err(e) => {
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
                self.active_pinned_search = Some(id);
                self.sidebar.active_pinned_search = Some(id);
                self.editing_pinned_search = Some(id);

                // Invalidate cached thread IDs for the updated pinned search
                for ps in &mut self.pinned_searches {
                    if ps.id == id {
                        ps.thread_ids = None;
                        break;
                    }
                }
                for ps in &mut self.sidebar.pinned_searches {
                    if ps.id == id {
                        ps.thread_ids = None;
                        break;
                    }
                }

                let db = Arc::clone(&self.db);
                Task::perform(
                    async move { db.list_pinned_searches().await },
                    Message::PinnedSearchesLoaded,
                )
            }
            Err(e) => {
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
                self.status = format!("Expiry warning: {e}");
                Task::none()
            }
        }
    }

    /// Refresh a pinned search by re-executing its query.
    pub(crate) fn handle_refresh_pinned_search(
        &mut self,
        id: i64,
    ) -> Task<Message> {
        // Invalidate cached thread IDs
        for ps in &mut self.pinned_searches {
            if ps.id == id {
                ps.thread_ids = None;
                break;
            }
        }
        for ps in &mut self.sidebar.pinned_searches {
            if ps.id == id {
                ps.thread_ids = None;
                break;
            }
        }

        // Find the query and re-execute
        let query = self
            .pinned_searches
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.query.clone());

        let Some(query) = query else {
            return Task::none();
        };

        // Set up pinned search context
        self.active_pinned_search = Some(id);
        self.sidebar.active_pinned_search = Some(id);
        self.editing_pinned_search = Some(id);
        self.search_query.clone_from(&query);
        self.thread_list.search_query.clone_from(&query);

        // Store pre-search threads if needed
        if self.thread_list.mode == ThreadListMode::Folder {
            self.pre_search_threads = Some(self.thread_list.threads.clone());
        }

        // Execute search
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

    /// Handle periodic expiry tick — run expiry if not recently checked.
    pub(crate) fn handle_expiry_tick(&mut self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.expire_stale_pinned_searches(1_209_600).await },
            Message::PinnedSearchesExpired,
        )
    }

    /// Handle "Search here" — prefill search bar with a scope query prefix.
    pub(crate) fn handle_search_here(
        &mut self,
        query_prefix: String,
    ) -> Task<Message> {
        // Store pre-search state
        if self.thread_list.mode == ThreadListMode::Folder {
            self.pre_search_threads = Some(self.thread_list.threads.clone());
        }

        self.search_query = query_prefix;
        self.thread_list.search_query.clone_from(&self.search_query);
        self.clear_pinned_search_context();

        // Focus the search bar so the user can type immediately
        iced::widget::operation::focus::<Message>("search-bar".to_string())
    }

    /// Handle "Save as Smart Folder" — create a smart folder from the
    /// current search query.
    pub(crate) fn handle_save_as_smart_folder(
        &mut self,
        name: String,
    ) -> Task<Message> {
        let query = self.search_query.trim().to_string();
        if query.is_empty() {
            self.status = "No search query to save".to_string();
            return Task::none();
        }

        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.create_smart_folder(name, query).await },
            Message::SmartFolderSaved,
        )
    }

    /// Handle smart folder saved result — reload navigation.
    pub(crate) fn handle_smart_folder_saved(
        &mut self,
        result: Result<i64, String>,
    ) -> Task<Message> {
        match result {
            Ok(_id) => {
                self.status = "Smart folder saved".to_string();
                // Reload navigation to show the new smart folder
                self.nav_generation += 1;
                self.fire_navigation_load()
            }
            Err(e) => {
                self.status = format!("Save smart folder error: {e}");
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
                        t.is_read, t.is_starred, t.is_pinned, t.is_muted,
                        t.has_attachments,
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
                    is_pinned: row.get(8)?,
                    is_muted: row.get(9)?,
                    has_attachments: row.get(10)?,
                    from_name: row.get(11)?,
                    from_address: row.get(12)?,
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
        is_pinned: false,
        is_muted: false,
        has_attachments: false,
        from_name: r.from_name,
        from_address: r.from_address,
    }
}
