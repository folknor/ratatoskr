use std::sync::Arc;

use iced::Task;

use crate::db::{self, Thread};
use crate::ui::sidebar::truncate_query;
use crate::ui::thread_list::ThreadListMode;
use crate::{App, Message};
use rtsk::db::types::AccountScope;
use rtsk::scope::ViewScope;

#[derive(Debug, Clone)]
pub(crate) enum SearchIntent {
    AdHoc { query: String, scope: ViewScope },
    SmartFolder { id: String, query: String },
    PinnedActivation { id: i64 },
    PinnedRefresh { id: i64 },
}

#[derive(Debug, Clone)]
struct UiSearchContext {
    current_scope: ViewScope,
    editing_pinned_search: Option<i64>,
    entered_from_folder_view: bool,
    pinned_searches: Vec<db::PinnedSearch>,
}

#[derive(Debug, Clone)]
pub(crate) enum SearchScope {
    View(ViewScope),
    QueryIntrinsic,
}

#[derive(Debug, Clone)]
pub(crate) enum SearchExecution {
    Query { query: String, scope: SearchScope },
    Snapshot { pinned_search_id: i64 },
}

#[derive(Debug, Clone)]
pub(crate) enum SearchPersistenceBehavior {
    None,
    CreatePinnedSnapshot { scope_account_id: Option<String> },
    UpdatePinnedSnapshot { id: i64, scope_account_id: Option<String> },
    RefreshPinnedSnapshot { id: i64, scope_account_id: Option<String> },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PinnedSearchRef {
    Existing(i64),
    FromPersistence,
}

#[derive(Debug, Clone)]
pub(crate) enum SearchPinnedStateBehavior {
    Clear,
    SmartFolder { id: String },
    PinnedSearch {
        active: PinnedSearchRef,
        editing: PinnedSearchRef,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SearchPostSuccessEffect {
    None,
    RefreshPinnedSearchList,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FolderRestoreBehavior {
    LeaveAsIs,
    EnterSearchFromFolderView,
}

#[derive(Debug, Clone)]
pub struct SearchCompletionBehavior {
    pub(crate) persistence: SearchPersistenceBehavior,
    pub(crate) pinned_state: SearchPinnedStateBehavior,
    pub(crate) post_success: SearchPostSuccessEffect,
    pub(crate) folder_restore: FolderRestoreBehavior,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedSearch {
    pub intent: SearchIntent,
    pub execution: SearchExecution,
    pub completion: SearchCompletionBehavior,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SearchFreshness {
    Query(rtsk::generation::GenerationToken<rtsk::generation::Search>),
    Snapshot(rtsk::generation::GenerationToken<rtsk::generation::Nav>),
}

#[derive(Debug, Clone)]
pub struct SearchExecutionResult {
    pub(crate) resolved: ResolvedSearch,
    pub(crate) freshness: SearchFreshness,
    pub(crate) results: Result<Vec<Thread>, String>,
}

fn resolve_search_intent(intent: SearchIntent, ctx: &UiSearchContext) -> ResolvedSearch {
    let folder_restore = if ctx.entered_from_folder_view {
        FolderRestoreBehavior::EnterSearchFromFolderView
    } else {
        FolderRestoreBehavior::LeaveAsIs
    };

    match intent.clone() {
        SearchIntent::AdHoc { query, scope } => {
            let scope_account_id = scope.account_id().map(ToOwned::to_owned);
            let (persistence, pinned_state) = if let Some(id) = ctx.editing_pinned_search {
                (
                    SearchPersistenceBehavior::UpdatePinnedSnapshot {
                        id,
                        scope_account_id,
                    },
                    SearchPinnedStateBehavior::PinnedSearch {
                        active: PinnedSearchRef::Existing(id),
                        editing: PinnedSearchRef::Existing(id),
                    },
                )
            } else {
                (
                    SearchPersistenceBehavior::CreatePinnedSnapshot { scope_account_id },
                    SearchPinnedStateBehavior::PinnedSearch {
                        active: PinnedSearchRef::FromPersistence,
                        editing: PinnedSearchRef::FromPersistence,
                    },
                )
            };

            ResolvedSearch {
                intent,
                execution: SearchExecution::Query {
                    query,
                    scope: SearchScope::View(scope),
                },
                completion: SearchCompletionBehavior {
                    persistence,
                    pinned_state,
                    post_success: SearchPostSuccessEffect::RefreshPinnedSearchList,
                    folder_restore,
                },
            }
        }
        SearchIntent::SmartFolder { id, query } => ResolvedSearch {
            intent,
            execution: SearchExecution::Query {
                query,
                scope: SearchScope::QueryIntrinsic,
            },
            completion: SearchCompletionBehavior {
                persistence: SearchPersistenceBehavior::None,
                pinned_state: SearchPinnedStateBehavior::SmartFolder { id },
                post_success: SearchPostSuccessEffect::None,
                folder_restore,
            },
        },
        SearchIntent::PinnedActivation { id } => ResolvedSearch {
            intent,
            execution: SearchExecution::Snapshot {
                pinned_search_id: id,
            },
            completion: SearchCompletionBehavior {
                persistence: SearchPersistenceBehavior::None,
                pinned_state: SearchPinnedStateBehavior::PinnedSearch {
                    active: PinnedSearchRef::Existing(id),
                    editing: PinnedSearchRef::Existing(id),
                },
                post_success: SearchPostSuccessEffect::None,
                folder_restore,
            },
        },
        SearchIntent::PinnedRefresh { id } => {
            let ps = ctx
                .pinned_searches
                .iter()
                .find(|ps| ps.id == id)
                .unwrap_or_else(|| panic!("Pinned refresh intent resolved without pinned search {id}"));
            let scope = ps
                .scope_account_id
                .as_ref()
                .map_or(AccountScope::All, |id| AccountScope::Single(id.clone()));
            let query = ps.query.clone();

            ResolvedSearch {
                intent,
                execution: SearchExecution::Query {
                    query,
                    scope: match scope {
                        AccountScope::All => SearchScope::View(ViewScope::AllAccounts),
                        AccountScope::Single(id) => SearchScope::View(ViewScope::Account(id)),
                        AccountScope::Multiple(_) => SearchScope::QueryIntrinsic,
                    },
                },
                completion: SearchCompletionBehavior {
                    persistence: SearchPersistenceBehavior::RefreshPinnedSnapshot {
                        id,
                        scope_account_id: ps.scope_account_id.clone(),
                    },
                    pinned_state: SearchPinnedStateBehavior::PinnedSearch {
                        active: PinnedSearchRef::Existing(id),
                        editing: PinnedSearchRef::Existing(id),
                    },
                    post_success: SearchPostSuccessEffect::RefreshPinnedSearchList,
                    folder_restore: FolderRestoreBehavior::LeaveAsIs,
                },
            }
        }
    }
}

fn execution_scope_to_account_scope(scope: &SearchScope) -> AccountScope {
    match scope {
        SearchScope::QueryIntrinsic => AccountScope::All,
        SearchScope::View(ViewScope::AllAccounts) => AccountScope::All,
        SearchScope::View(ViewScope::Account(id)) => AccountScope::Single(id.clone()),
        SearchScope::View(ViewScope::SharedMailbox { account_id, .. })
        | SearchScope::View(ViewScope::PublicFolder { account_id, .. }) => {
            AccountScope::Single(account_id.clone())
        }
    }
}

// ── Search handling ────────────────────────────────────

impl App {
    fn record_search_restore_origin(&mut self) {
        if self.thread_list.mode == ThreadListMode::Folder {
            self.was_in_folder_view = true;
        }
    }

    fn current_ui_search_context(&self) -> UiSearchContext {
        UiSearchContext {
            current_scope: self.current_scope().clone(),
            editing_pinned_search: self.editing_pinned_search,
            entered_from_folder_view: self.thread_list.mode == ThreadListMode::Folder,
            pinned_searches: self.pinned_searches.clone(),
        }
    }

    fn apply_search_folder_restore(&mut self, behavior: FolderRestoreBehavior) {
        if matches!(behavior, FolderRestoreBehavior::EnterSearchFromFolderView) {
            self.record_search_restore_origin();
        }
    }

    fn dispatch_resolved_search(&mut self, resolved: ResolvedSearch) -> Task<Message> {
        self.apply_search_folder_restore(resolved.completion.folder_restore);

        match resolved.execution.clone() {
            SearchExecution::Query { query, scope } => {
                let generation = self.search_generation.next();
                let db = Arc::clone(&self.db);
                let ss = self.search_state.clone();
                let scope = execution_scope_to_account_scope(&scope);
                let resolved = resolved.clone();

                Task::perform(
                    async move {
                        let result = execute_search(db, ss, query, scope).await;
                        SearchExecutionResult {
                            resolved,
                            freshness: SearchFreshness::Query(generation),
                            results: result,
                        }
                    },
                    Message::SearchCompleted,
                )
            }
            SearchExecution::Snapshot { pinned_search_id } => {
                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation.next();
                let resolved = resolved.clone();
                Task::perform(
                    async move {
                        let result = match db.get_pinned_search_thread_ids(pinned_search_id).await {
                            Ok(ids) => db.get_threads_by_ids(ids).await,
                            Err(e) => Err(e),
                        };
                        SearchExecutionResult {
                            resolved,
                            freshness: SearchFreshness::Snapshot(load_gen),
                            results: result,
                        }
                    },
                    Message::SearchCompleted,
                )
            }
        }
    }

    pub(crate) fn handle_search_query_changed(&mut self, query: String) -> Task<Message> {
        self.search_query.set_text(query);
        self.thread_list.search_query = self.search_query.text().to_string();
        if self.search_query.text().trim().is_empty() {
            self.search_debounce_deadline = None;
            self.thread_list.typeahead.visible = false;
            if self.thread_list.mode == ThreadListMode::Search {
                let _ = self.nav_generation.next();
                let _ = self.search_generation.next();
                return self.restore_folder_view();
            }
        } else {
            self.search_debounce_deadline =
                Some(iced::time::Instant::now() + std::time::Duration::from_millis(150));
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

        let intent = SearchIntent::AdHoc {
            query,
            scope: self.current_ui_search_context().current_scope.clone(),
        };
        let resolved = resolve_search_intent(intent, &self.current_ui_search_context());
        self.dispatch_resolved_search(resolved)
    }

    pub(crate) fn handle_search_completed(
        &mut self,
        result: SearchExecutionResult,
    ) -> Task<Message> {
        let is_fresh = match result.freshness {
            SearchFreshness::Query(token) => self.search_generation.is_current(token),
            SearchFreshness::Snapshot(token) => self.nav_generation.is_current(token),
        };
        if !is_fresh {
            return Task::none();
        }

        match result.results {
            Ok(threads) => {
                self.thread_list.mode = ThreadListMode::Search;

                self.thread_list.set_threads(threads);
                self.clear_thread_selection();

                match &result.resolved.intent {
                    SearchIntent::PinnedActivation { id } | SearchIntent::PinnedRefresh { id } => {
                        if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == *id) {
                            let label = truncate_query(&ps.query, 30);
                            self.search_query.reset(ps.query.clone());
                            self.thread_list.search_query.clone_from(&ps.query);
                            self.thread_list.set_context(
                                format!("Search: {label}"),
                                pinned_search_scope_name(self, ps),
                            );
                            self.status = format!("{} threads (pinned search)", self.thread_list.threads.len());
                        }
                    }
                    _ => {
                        self.status = format!("{} results", self.thread_list.threads.len());
                    }
                }

                let thread_ids: Vec<(String, String)> = self
                    .thread_list
                    .threads
                    .iter()
                    .map(|t| (t.id.clone(), t.account_id.clone()))
                    .collect();

                let persistence_task = match (&result.resolved.completion.persistence, &result.resolved.execution) {
                    (SearchPersistenceBehavior::None, _) => Task::none(),
                    (
                        SearchPersistenceBehavior::CreatePinnedSnapshot { scope_account_id },
                        SearchExecution::Query { query, .. },
                    ) => {
                        let db = Arc::clone(&self.db);
                        let query = query.clone();
                        let scope_account_id = scope_account_id.clone();
                        let completion = result.resolved.completion.clone();
                        Task::perform(
                            async move {
                                db.create_or_update_pinned_search(query, thread_ids, scope_account_id)
                                    .await
                            },
                            move |save_result| Message::PinnedSearchPersisted(completion.clone(), save_result),
                        )
                    }
                    (
                        SearchPersistenceBehavior::UpdatePinnedSnapshot { id, scope_account_id },
                        SearchExecution::Query { query, .. },
                    ) => {
                        let db = Arc::clone(&self.db);
                        let query = query.clone();
                        let scope_account_id = scope_account_id.clone();
                        let id = *id;
                        let completion = result.resolved.completion.clone();
                        Task::perform(
                            async move {
                                db.update_pinned_search(id, query, thread_ids, scope_account_id)
                                    .await
                                    .map(|()| id)
                            },
                            move |save_result| Message::PinnedSearchPersisted(completion.clone(), save_result),
                        )
                    }
                    (
                        SearchPersistenceBehavior::RefreshPinnedSnapshot { id, scope_account_id },
                        SearchExecution::Query { query, .. },
                    ) => {
                        let db = Arc::clone(&self.db);
                        let query = query.clone();
                        let scope_account_id = scope_account_id.clone();
                        let id = *id;
                        let completion = result.resolved.completion.clone();
                        Task::perform(
                            async move {
                                db.update_pinned_search(id, query, thread_ids, scope_account_id)
                                    .await
                                    .map(|()| id)
                            },
                            move |save_result| Message::PinnedSearchPersisted(completion.clone(), save_result),
                        )
                    }
                    _ => Task::none(),
                };

                if matches!(result.resolved.completion.persistence, SearchPersistenceBehavior::None)
                {
                    self.apply_search_pinned_state(&result.resolved.completion.pinned_state, None);
                    return self.apply_search_post_success(result.resolved.completion.post_success);
                }

                Task::batch([persistence_task])
            }
            Err(e) => {
                self.status = format!("Search error: {e}");
                Task::none()
            }
        }
    }

    fn apply_search_pinned_state(
        &mut self,
        behavior: &SearchPinnedStateBehavior,
        persisted_id: Option<i64>,
    ) {
        match behavior {
            SearchPinnedStateBehavior::Clear => {
                self.sidebar.active_pinned_search = None;
                self.editing_pinned_search = None;
            }
            SearchPinnedStateBehavior::SmartFolder { .. } => {
                self.sidebar.active_pinned_search = None;
                self.editing_pinned_search = None;
            }
            SearchPinnedStateBehavior::PinnedSearch { active, editing } => {
                let resolve_ref = |r: PinnedSearchRef, persisted_id: Option<i64>| match r {
                    PinnedSearchRef::Existing(id) => Some(id),
                    PinnedSearchRef::FromPersistence => persisted_id,
                };
                self.sidebar.active_pinned_search = resolve_ref(*active, persisted_id);
                self.editing_pinned_search = resolve_ref(*editing, persisted_id);
            }
        }
    }

    fn apply_search_post_success(&mut self, effect: SearchPostSuccessEffect) -> Task<Message> {
        match effect {
            SearchPostSuccessEffect::None => Task::none(),
            SearchPostSuccessEffect::RefreshPinnedSearchList => {
                let db = Arc::clone(&self.db);
                Task::perform(async move { db.list_pinned_searches().await }, Message::PinnedSearchesLoaded)
            }
        }
    }

    pub(crate) fn apply_search_debounce(&mut self) -> Task<Message> {
        if self.search_query.text().trim().is_empty() {
            self.search_debounce_deadline = None;
        } else {
            self.search_debounce_deadline =
                Some(iced::time::Instant::now() + std::time::Duration::from_millis(150));
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
        id: String,
        query: String,
    ) -> Task<Message> {
        self.clear_pinned_search_context();
        self.search_query.set_text(query.clone());
        self.thread_list.search_query = query;
        let intent = SearchIntent::SmartFolder {
            id,
            query: self.search_query.text().to_string(),
        };
        let resolved = resolve_search_intent(intent, &self.current_ui_search_context());
        self.dispatch_resolved_search(resolved)
    }

    /// Dispatch an async typeahead query based on operator type.
    fn dispatch_typeahead_query(&mut self, operator: &str, partial: &str) -> Task<Message> {
        use crate::ui::thread_list::{ThreadListMessage, TypeaheadItem};

        let load_gen = self.thread_list.typeahead.generation.next();

        // Static operators — resolve immediately
        let static_items: Option<Vec<TypeaheadItem>> = match operator {
            "in" => Some(
                [
                    "inbox", "sent", "drafts", "trash", "spam", "starred", "snoozed",
                ]
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
                [
                    "unread", "read", "starred", "snoozed", "pinned", "muted", "tagged",
                ]
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
                [
                    "attachment",
                    "pdf",
                    "image",
                    "excel",
                    "word",
                    "document",
                    "archive",
                    "video",
                    "audio",
                    "powerpoint",
                    "spreadsheet",
                    "calendar",
                    "contact",
                ]
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
                [
                    ("Today", "0"),
                    ("Yesterday", "-1"),
                    ("Last 7 days", "-7"),
                    ("Last 30 days", "-30"),
                    ("Last 3 months", "-90"),
                    ("Last year", "-365"),
                ]
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
                    "from" | "to" => db.search_contacts_for_typeahead(partial).await,
                    "label" | "folder" => db.search_labels_for_typeahead(partial).await,
                    "account" => db.search_accounts_for_typeahead(partial).await,
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
            self.search_debounce_deadline =
                Some(iced::time::Instant::now() + std::time::Duration::from_millis(50));
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
                self.sidebar
                    .pinned_searches
                    .clone_from(&self.pinned_searches);

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
        let _ = self.nav_generation.next();
        let _ = self.thread_generation.next();
        self.clear_thread_selection();
        let intent = SearchIntent::PinnedActivation { id };
        let resolved = resolve_search_intent(intent, &self.current_ui_search_context());
        self.dispatch_resolved_search(resolved)
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

    pub(crate) fn handle_pinned_search_persisted(
        &mut self,
        completion: SearchCompletionBehavior,
        result: Result<i64, String>,
    ) -> Task<Message> {
        match result {
            Ok(id) => {
                self.apply_search_pinned_state(&completion.pinned_state, Some(id));
                self.apply_search_post_success(completion.post_success)
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
        self.clear_pinned_search_context();
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
    scope: rtsk::db::types::AccountScope,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        if let Some(ref ss) = search_state {
            let results = rtsk::search_pipeline::search(&query, ss, conn)?;
            Ok(results
                .into_iter()
                .map(unified_result_to_thread)
                .filter(|thread| thread_matches_scope(thread, &scope))
                .collect())
        } else {
            execute_search_sql_fallback(conn, &query, &scope)
        }
    })
    .await
}

/// SQL-only fallback search using the smart folder parser and SQL builder.
fn execute_search_sql_fallback(
    conn: &rusqlite::Connection,
    query: &str,
    scope: &rtsk::db::types::AccountScope,
) -> Result<Vec<Thread>, String> {
    let parsed = rtsk::smart_folder::parse_query(query);
    let scope = scope.clone();

    if parsed.has_any_operator() || parsed.free_text.is_empty() {
        let db_threads =
            rtsk::smart_folder::query_threads(conn, &parsed, &scope, Some(200), Some(0))?;
        Ok(db_threads
            .into_iter()
            .map(crate::db_thread_to_app_thread)
            .collect())
    } else {
        // Free text only, no Tantivy — do a simple LIKE search
        let pattern = format!("%{}%", parsed.free_text);
        let (scope_clause, scope_params): (String, Vec<String>) = match &scope {
            rtsk::db::types::AccountScope::All => (String::new(), vec![]),
            rtsk::db::types::AccountScope::Single(id) => {
                ("AND t.account_id = ?2".to_string(), vec![id.clone()])
            }
            rtsk::db::types::AccountScope::Multiple(ids) => {
                let placeholders: Vec<String> =
                    (0..ids.len()).map(|i| format!("?{}", i + 2)).collect();
                (
                    format!("AND t.account_id IN ({})", placeholders.join(",")),
                    ids.clone(),
                )
            }
        };
        let sql = format!(
            "SELECT t.id, t.account_id, t.subject, t.snippet,
                    t.last_message_at, t.message_count,
                    t.is_read, t.is_starred, t.has_attachments,
                    t.from_name, t.from_address
             FROM threads t
             WHERE (t.subject LIKE ?1 OR t.snippet LIKE ?1)
             {scope_clause}
             ORDER BY t.last_message_at DESC
             LIMIT 200"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare search: {e}"))?;
        let row_mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Thread> {
            Ok(Thread {
                id: row.get(0)?,
                account_id: row.get(1)?,
                subject: row.get(2)?,
                snippet: row.get(3)?,
                last_message_at: row
                    .get::<_, Option<String>>(4)?
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
        };
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(pattern)];
        for p in &scope_params {
            params.push(Box::new(p.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(&*param_refs, row_mapper)
            .map_err(|e| format!("search query: {e}"))?;
        let mut threads = Vec::new();
        for row in rows {
            threads.push(row.map_err(|e| format!("search row: {e}"))?);
        }
        Ok(threads)
    }
}

/// Convert a `UnifiedSearchResult` from the search pipeline to an app `Thread`.
fn unified_result_to_thread(r: rtsk::search_pipeline::UnifiedSearchResult) -> Thread {
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

fn thread_matches_scope(thread: &Thread, scope: &AccountScope) -> bool {
    match scope {
        AccountScope::All => true,
        AccountScope::Single(id) => thread.account_id == *id,
        AccountScope::Multiple(ids) => ids.iter().any(|id| thread.account_id == *id),
    }
}

fn pinned_search_scope_name(app: &App, ps: &db::PinnedSearch) -> String {
    let Some(account_id) = ps.scope_account_id.as_deref() else {
        return "All Accounts".to_string();
    };

    app.sidebar
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .map(|account| {
            account
                .account_name
                .clone()
                .or_else(|| account.display_name.clone())
                .unwrap_or_else(|| account.email.clone())
        })
        .unwrap_or_else(|| "All Accounts".to_string())
}

// ── Search phases 2+4 methods ──────────────────────────

impl App {
    pub(crate) fn handle_refresh_pinned_search(&mut self, id: i64) -> Task<Message> {
        if !self.pinned_searches.iter().any(|ps| ps.id == id) {
            return Task::none();
        }

        let intent = SearchIntent::PinnedRefresh { id };
        let resolved = resolve_search_intent(intent, &self.current_ui_search_context());
        self.dispatch_resolved_search(resolved)
    }

    pub(crate) fn handle_expiry_tick(&mut self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.expire_stale_pinned_searches(1_209_600).await },
            Message::PinnedSearchesExpired,
        )
    }

    pub(crate) fn handle_search_here(&mut self, query_prefix: String) -> Task<Message> {
        self.record_search_restore_origin();
        self.search_query.reset(query_prefix.clone());
        self.thread_list.search_query = query_prefix;
        self.clear_pinned_search_context();
        iced::widget::operation::focus::<Message>("search-bar".to_string())
    }

    pub(crate) fn handle_save_as_smart_folder(&mut self, name: String) -> Task<Message> {
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
