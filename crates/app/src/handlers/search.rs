use std::sync::Arc;

use iced::Task;

use crate::db::{self, Thread};
use crate::ui::sidebar::truncate_query;
use crate::ui::thread_list::ThreadListMode;
use crate::{App, Message};

// ── Search handling ────────────────────────────────────

impl App {
    pub(crate) fn handle_search_query_changed(&mut self, query: String) -> Task<Message> {
        self.search_query.set_text(query);
        self.thread_list.search_query = self.search_query.text().to_string();
        if self.search_query.text().trim().is_empty() {
            self.search_debounce_deadline = None;
            self.thread_list.typeahead.visible = false;
            if self.thread_list.mode == ThreadListMode::Search {
                self.clear_pinned_search_context();
                let _ = self.nav_generation.next();
                let _ = self.search_generation.next();
                return self.restore_folder_view();
            }
        } else {
            self.search_debounce_deadline = Some(
                iced::time::Instant::now()
                    + std::time::Duration::from_millis(150),
            );
        }

        // Trigger typeahead based on cursor context (assume cursor at end)
        let text = self.search_query.text().to_string();
        let cursor_pos = text.len();
        let ctx = smart_folder::analyze_cursor_context(&text, cursor_pos);
        match ctx {
            smart_folder::CursorContext::InsideOperator {
                operator,
                partial_value,
                ..
            } if !partial_value.is_empty() => {
                return self.dispatch_typeahead_query(&operator, &partial_value);
            }
            _ => {
                self.thread_list.typeahead.visible = false;
            }
        }
        Task::none()
    }

    pub(crate) fn handle_search_execute(&mut self) -> Task<Message> {
        self.search_debounce_deadline = None;
        let query = self.search_query.text().trim().to_string();
        if query.is_empty() {
            return self.restore_folder_view();
        }

        // Remember that we were in folder mode before searching
        if self.thread_list.mode == ThreadListMode::Folder {
            self.was_in_folder_view = true;
        }

        let generation = self.search_generation.next();
        let db = Arc::clone(&self.db);
        let ss = self.search_state.clone();

        Task::perform(
            async move {
                let result = execute_search(db, ss, query).await;
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
                let query = self.search_query.text().to_string();

                self.thread_list.set_threads(threads);
                self.clear_thread_selection();

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

    pub(crate) fn apply_search_debounce(&mut self) -> Task<Message> {
        if self.search_query.text().trim().is_empty() {
            self.search_debounce_deadline = None;
        } else {
            self.search_debounce_deadline = Some(
                iced::time::Instant::now() + std::time::Duration::from_millis(150),
            );
        }
        Task::none()
    }

    pub(crate) fn handle_search_clear(&mut self) -> Task<Message> {
        self.search_query.reset(String::new());
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        let _ = self.search_generation.next();
        self.restore_folder_view()
    }

    pub(crate) fn handle_focus_search_bar(&self) -> Task<Message> {
        iced::widget::operation::focus::<Message>("search-bar".to_string())
    }

    /// Handle smart folder selection from the sidebar — fill the search bar
    /// and execute the query via the unified search pipeline.
    pub(crate) fn handle_smart_folder_selected(
        &mut self,
        _id: String,
        query: String,
    ) -> Task<Message> {
        self.search_query.set_text(query.clone());
        self.thread_list.search_query = query;

        if self.thread_list.mode == ThreadListMode::Folder {
            self.was_in_folder_view = true;
        }

        let generation = self.search_generation.next();
        let db = Arc::clone(&self.db);
        let ss = self.search_state.clone();

        let search_query = self.search_query.text().to_string();
        Task::perform(
            async move {
                let result = execute_search(db, ss, search_query).await;
                (generation, result)
            },
            |(g, result)| Message::SearchResultsLoaded(g, result),
        )
    }

    /// Dispatch an async typeahead query based on operator type.
    fn dispatch_typeahead_query(
        &mut self,
        operator: &str,
        partial: &str,
    ) -> Task<Message> {
        use crate::ui::thread_list::{TypeaheadItem, ThreadListMessage};

        let load_gen = self.thread_list.typeahead.generation.next();

        // Static operators — resolve immediately
        let static_items: Option<Vec<TypeaheadItem>> = match operator {
            "in" => Some(
                ["inbox", "sent", "drafts", "trash", "spam", "starred", "snoozed"]
                    .iter()
                    .filter(|s| s.starts_with(&partial.to_lowercase()))
                    .map(|s| TypeaheadItem {
                        label: s.to_string(),
                        detail: None,
                        insert_value: s.to_string(),
                    })
                    .collect(),
            ),
            "is" => Some(
                ["unread", "read", "starred", "snoozed", "pinned", "muted", "tagged"]
                    .iter()
                    .filter(|s| s.starts_with(&partial.to_lowercase()))
                    .map(|s| TypeaheadItem {
                        label: s.to_string(),
                        detail: None,
                        insert_value: s.to_string(),
                    })
                    .collect(),
            ),
            "has" => Some(
                ["attachment", "pdf", "image", "excel", "word", "document", "archive", "video", "audio", "powerpoint", "spreadsheet", "calendar", "contact"]
                    .iter()
                    .filter(|s| s.starts_with(&partial.to_lowercase()))
                    .map(|s| TypeaheadItem {
                        label: s.to_string(),
                        detail: None,
                        insert_value: s.to_string(),
                    })
                    .collect(),
            ),
            "before" | "after" => Some(
                [("Today", "0"), ("Yesterday", "-1"), ("Last 7 days", "-7"),
                 ("Last 30 days", "-30"), ("Last 3 months", "-90"), ("Last year", "-365")]
                    .iter()
                    .filter(|(label, _)| label.to_lowercase().contains(&partial.to_lowercase()))
                    .map(|(label, value)| TypeaheadItem {
                        label: label.to_string(),
                        detail: Some(format!("{operator}:{value}")),
                        insert_value: value.to_string(),
                    })
                    .collect(),
            ),
            _ => None,
        };

        if let Some(items) = static_items {
            return Task::done(Message::ThreadList(
                ThreadListMessage::TypeaheadItemsLoaded(load_gen, items),
            ));
        }

        // Dynamic operators — query DB asynchronously
        let db = Arc::clone(&self.db);
        let partial = partial.to_string();
        let op = operator.to_string();
        Task::perform(
            async move {
                match op.as_str() {
                    "from" | "to" => {
                        db.search_contacts_for_typeahead(partial).await
                    }
                    "label" | "folder" => {
                        db.search_labels_for_typeahead(partial).await
                    }
                    "account" => {
                        db.search_accounts_for_typeahead(partial).await
                    }
                    _ => Ok(Vec::new()),
                }
            },
            move |result| {
                let items = result.unwrap_or_default();
                Message::ThreadList(ThreadListMessage::TypeaheadItemsLoaded(load_gen, items))
            },
        )
    }

    /// Handle typeahead selection — insert the value into the query.
    pub(crate) fn handle_typeahead_select(&mut self, idx: usize) -> Task<Message> {
        let Some(item) = self.thread_list.typeahead.items.get(idx) else {
            return Task::none();
        };

        let query = self.search_query.text().to_string();
        let cursor_pos = query.len();
        let ctx = smart_folder::analyze_cursor_context(&query, cursor_pos);

        if let smart_folder::CursorContext::InsideOperator {
            value_start,
            value_end,
            ..
        } = ctx
        {
            let value = if item.insert_value.contains(' ') {
                format!("\"{}\" ", item.insert_value)
            } else {
                format!("{} ", item.insert_value)
            };
            let new_query = format!("{}{}{}", &query[..value_start], value, &query[value_end..]);
            self.search_query.set_text(new_query.clone());
            self.thread_list.search_query = new_query;
            self.thread_list.typeahead.visible = false;

            // Trigger search execution with the new query
            self.search_debounce_deadline = Some(
                iced::time::Instant::now() + std::time::Duration::from_millis(50),
            );
        }
        Task::none()
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

                let db = Arc::clone(&self.db);
                let history_task = Task::perform(
                    async move { db.get_recent_search_queries(10).await },
                    Message::SearchHistoryLoaded,
                );

                if !self.expiry_ran {
                    self.expiry_ran = true;
                    let db2 = Arc::clone(&self.db);
                    return Task::batch([
                        history_task,
                        Task::perform(
                            async move { db2.expire_stale_pinned_searches(1_209_600).await },
                            Message::PinnedSearchesExpired,
                        ),
                    ]);
                }
                history_task
            }
            Err(e) => {
                self.status = format!("Pinned searches error: {e}");
                Task::none()
            }
        }
    }

    pub(crate) fn handle_select_pinned_search(&mut self, id: i64) -> Task<Message> {
        // Remember that we were in folder mode before switching to pinned search
        if self.sidebar.active_pinned_search.is_none()
            && self.thread_list.mode == ThreadListMode::Folder
        {
            self.was_in_folder_view = true;
        }

        self.sidebar.active_pinned_search = Some(id);
        self.editing_pinned_search = Some(id);
        self.sidebar.selected_label = None;

        let _ = self.nav_generation.next();
        let _ = self.thread_generation.next();
        self.clear_thread_selection();

        // Update thread list context
        if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == id) {
            let label = truncate_query(&ps.query, 30);
            self.thread_list
                .set_context(format!("Search: {label}"), "All Accounts".to_string());
        }

        let db = Arc::clone(&self.db);
        let load_gen = self.nav_generation.next();
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
                    self.search_query.reset(ps.query.clone());
                    self.thread_list.search_query.clone_from(&ps.query);
                }

                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation.next();
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
                self.clear_thread_selection();
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
                if self.sidebar.active_pinned_search == Some(id) {
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
                self.sidebar.active_pinned_search = Some(id);
                self.editing_pinned_search = Some(id);

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

    /// Clear pinned search context on navigate-away.
    pub(crate) fn clear_pinned_search_context(&mut self) {
        self.sidebar.active_pinned_search = None;
        self.editing_pinned_search = None;
    }

    /// Clear all search-related state without restoring pre-search threads.
    pub(crate) fn clear_search_state(&mut self) {
        self.search_query.reset(String::new());
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        let _ = self.search_generation.next();
        self.thread_list.mode = ThreadListMode::Folder;
        self.was_in_folder_view = false;
    }

    /// Restore the thread list to folder view after clearing search.
    ///
    /// Instead of restoring a stale clone of the pre-search thread list,
    /// this reloads from the database using the current navigation state.
    /// This avoids the O(n) clone on every search entry and ensures the
    /// thread list reflects any changes that happened during the search.
    pub(crate) fn restore_folder_view(&mut self) -> Task<Message> {
        self.thread_list.mode = ThreadListMode::Folder;
        self.search_query.reset(String::new());
        self.thread_list.search_query.clear();
        self.clear_thread_selection();
        if self.was_in_folder_view {
            self.was_in_folder_view = false;
            let token = self.nav_generation.next();
            return self.load_threads_for_current_view(token);
        }
        Task::none()
    }
}

/// Execute search off the main thread via spawn_blocking.
///
/// Uses the provided SearchState (initialized once at boot) for Tantivy.
/// Falls back to SQL-only if no SearchState is available.
pub(crate) async fn execute_search(
    db: Arc<db::Db>,
    search_state: Option<Arc<rtsk::search::SearchState>>,
    query: String,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        if let Some(ref ss) = search_state {
            let results = rtsk::search_pipeline::search(
                &query, ss, conn,
            )?;
            Ok(results.into_iter().map(unified_result_to_thread).collect())
        } else {
            execute_search_sql_fallback(conn, &query)
        }
    })
    .await
}

/// SQL-only fallback search using the smart folder parser and SQL builder.
fn execute_search_sql_fallback(
    conn: &rusqlite::Connection,
    query: &str,
) -> Result<Vec<Thread>, String> {
    let parsed = rtsk::smart_folder::parse_query(query);
    let scope = rtsk::db::types::AccountScope::All;

    if parsed.has_any_operator() || parsed.free_text.is_empty() {
        let db_threads = rtsk::smart_folder::query_threads(
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
                    is_pinned: false,
                    is_muted: false,
                    has_attachments: row.get(8)?,
                    from_name: row.get(9)?,
                    from_address: row.get(10)?,
                    is_local_draft: false,
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
    r: rtsk::search_pipeline::UnifiedSearchResult,
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
        is_local_draft: false,
    }
}

// ── Search phases 2+4 methods ──────────────────────────

impl App {
    pub(crate) fn handle_refresh_pinned_search(
        &mut self,
        id: i64,
    ) -> Task<Message> {
        for ps in &mut self.pinned_searches {
            if ps.id == id {
                ps.thread_ids = None;
                break;
            }
        }
        self.handle_select_pinned_search(id)
    }

    pub(crate) fn handle_expiry_tick(&mut self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.expire_stale_pinned_searches(1_209_600).await },
            Message::PinnedSearchesExpired,
        )
    }

    pub(crate) fn handle_search_here(
        &mut self,
        query_prefix: String,
    ) -> Task<Message> {
        if self.thread_list.mode == ThreadListMode::Folder {
            self.was_in_folder_view = true;
        }
        self.search_query.reset(query_prefix.clone());
        self.thread_list.search_query = query_prefix;
        self.clear_pinned_search_context();
        iced::widget::operation::focus::<Message>("search-bar".to_string())
    }

    pub(crate) fn handle_save_as_smart_folder(
        &mut self,
        name: String,
    ) -> Task<Message> {
        let query = self.search_query.text().trim().to_string();
        if query.is_empty() {
            return Task::none();
        }
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.create_smart_folder(name, query).await },
            Message::SmartFolderSaved,
        )
    }

    pub(crate) fn handle_smart_folder_saved(
        &mut self,
        result: Result<i64, String>,
    ) -> Task<Message> {
        match result {
            Ok(_id) => {
                log::info!("Smart folder saved");
                let token = self.nav_generation.next();
                self.fire_navigation_load(token)
            }
            Err(e) => {
                log::error!("Save smart folder error: {e}");
                Task::none()
            }
        }
    }
}
