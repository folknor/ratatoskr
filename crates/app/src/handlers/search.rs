use std::sync::Arc;

use iced::Task;
use service_api::{PinnedSearchCreateOrUpdateParams, PinnedSearchUpdateParams, PinnedThreadRef};

use crate::db::{self, Thread};
use crate::ui::sidebar::truncate_query;
use crate::ui::thread_list::ThreadListMode;
use crate::{Message, ReadyApp};
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
    entered_from_scope_view: bool,
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
#[allow(dead_code)] // Clear variant + id field reserved for upcoming pinned-search edits
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
    let folder_restore = if ctx.entered_from_scope_view {
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

impl ReadyApp {
    fn record_search_restore_origin(&mut self) {
        if self.thread_list.mode == ThreadListMode::Scope {
            self.was_in_scope_view = true;
        }
    }

    fn current_ui_search_context(&self) -> UiSearchContext {
        UiSearchContext {
            current_scope: self.current_scope().clone(),
            editing_pinned_search: self.editing_pinned_search,
            entered_from_scope_view: self.thread_list.mode == ThreadListMode::Scope,
            pinned_searches: self.pinned_searches.clone(),
        }
    }

    fn apply_search_folder_restore(&mut self, behavior: FolderRestoreBehavior) {
        if matches!(behavior, FolderRestoreBehavior::EnterSearchFromFolderView) {
            self.record_search_restore_origin();
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub(crate) fn dispatch_resolved_search(&mut self, resolved: ResolvedSearch) -> Task<Message> {
        // Pre-boot race: SearchReadState::init is still in flight. Stash the
        // resolved intent so SearchStateReady can replay it once init lands,
        // rather than letting the search pipeline fall through to the SQL
        // LIKE fallback. The fallback is reserved for true init-failure
        // recovery (which stays an open design question - see
        // `docs/search/implementation-spec.md` § "SQL fallback"). Only the
        // Query variant uses the search index; Snapshot loads stored thread
        // IDs from `pinned_search_threads` and is safe to run pre-boot.
        if self.search_state_pending
            && matches!(resolved.execution, SearchExecution::Query { .. })
        {
            self.pending_search = Some(resolved);
            return Task::none();
        }

        self.apply_search_folder_restore(resolved.completion.folder_restore);

        match resolved.execution.clone() {
            SearchExecution::Query { query, scope } => {
                let generation = self.search_generation.next();
                let db = Arc::clone(&self.db);
                let ss = self.search_state.clone();
                let body_store = self.body_store.clone();
                let scope = execution_scope_to_account_scope(&scope);
                let resolved = resolved.clone();

                Task::perform(
                    async move {
                        let result = execute_search(db, ss, body_store, query, scope).await;
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
                        if let Some(client) = self.service_client.as_ref().cloned() {
                            let query = query.clone();
                            let scope_account_id = scope_account_id.clone();
                            let completion = result.resolved.completion.clone();
                            let params = PinnedSearchCreateOrUpdateParams {
                                query,
                                thread_ids: thread_ids
                                    .into_iter()
                                    .map(|(thread_id, account_id)| PinnedThreadRef {
                                        thread_id,
                                        account_id,
                                    })
                                    .collect(),
                                scope_account_id,
                            };
                            Task::perform(
                                async move {
                                    client
                                        .create_or_update_pinned_search(params)
                                        .await
                                        .map_err(|e| e.to_string())
                                },
                                move |save_result| {
                                    Message::PinnedSearchCreateOrUpdateAck(
                                        completion.clone(),
                                        save_result,
                                    )
                                },
                            )
                        } else {
                            log::warn!(
                                "pinned_search.create_or_update: no ServiceClient yet; \
                                 snapshot not persisted (next reload reconciles)"
                            );
                            Task::none()
                        }
                    }
                    (
                        SearchPersistenceBehavior::UpdatePinnedSnapshot { id, scope_account_id },
                        SearchExecution::Query { query, .. },
                    ) => {
                        if let Some(client) = self.service_client.as_ref().cloned() {
                            let query = query.clone();
                            let scope_account_id = scope_account_id.clone();
                            let id = *id;
                            let completion = result.resolved.completion.clone();
                            let params = PinnedSearchUpdateParams {
                                id,
                                query,
                                thread_ids: thread_ids
                                    .into_iter()
                                    .map(|(thread_id, account_id)| PinnedThreadRef {
                                        thread_id,
                                        account_id,
                                    })
                                    .collect(),
                                scope_account_id,
                            };
                            Task::perform(
                                async move {
                                    client
                                        .update_pinned_search(params)
                                        .await
                                        .map_err(|e| e.to_string())
                                        .map(|()| id)
                                },
                                move |save_result| {
                                    Message::PinnedSearchUpdateAck(
                                        completion.clone(),
                                        save_result,
                                    )
                                },
                            )
                        } else {
                            log::warn!(
                                "pinned_search.update: no ServiceClient yet; \
                                 snapshot not persisted (next reload reconciles)"
                            );
                            Task::none()
                        }
                    }
                    (
                        SearchPersistenceBehavior::RefreshPinnedSnapshot { id, scope_account_id },
                        SearchExecution::Query { query, .. },
                    ) => {
                        if let Some(client) = self.service_client.as_ref().cloned() {
                            let query = query.clone();
                            let scope_account_id = scope_account_id.clone();
                            let id = *id;
                            let completion = result.resolved.completion.clone();
                            let params = PinnedSearchUpdateParams {
                                id,
                                query,
                                thread_ids: thread_ids
                                    .into_iter()
                                    .map(|(thread_id, account_id)| PinnedThreadRef {
                                        thread_id,
                                        account_id,
                                    })
                                    .collect(),
                                scope_account_id,
                            };
                            Task::perform(
                                async move {
                                    client
                                        .update_pinned_search(params)
                                        .await
                                        .map_err(|e| e.to_string())
                                        .map(|()| id)
                                },
                                move |save_result| {
                                    Message::PinnedSearchUpdateAck(
                                        completion.clone(),
                                        save_result,
                                    )
                                },
                            )
                        } else {
                            log::warn!(
                                "pinned_search.update (refresh): no ServiceClient yet; \
                                 snapshot not persisted (next reload reconciles)"
                            );
                            Task::none()
                        }
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
                self.thread_list.pinned_search_updated_at = None;
            }
            SearchPinnedStateBehavior::SmartFolder { .. } => {
                self.sidebar.active_pinned_search = None;
                self.editing_pinned_search = None;
                self.thread_list.pinned_search_updated_at = None;
            }
            SearchPinnedStateBehavior::PinnedSearch { active, editing } => {
                let resolve_ref = |r: PinnedSearchRef, persisted_id: Option<i64>| match r {
                    PinnedSearchRef::Existing(id) => Some(id),
                    PinnedSearchRef::FromPersistence => persisted_id,
                };
                let active_id = resolve_ref(*active, persisted_id);
                self.sidebar.active_pinned_search = active_id;
                self.editing_pinned_search = resolve_ref(*editing, persisted_id);
                // Mirror the active pinned search's `updated_at` onto
                // the thread list so the "Last updated …" indicator
                // under the search bar can render. Fall back to "now"
                // for freshly created rows whose `pinned_searches`
                // entry has not yet round-tripped through the list
                // reload - the subsequent `handle_pinned_searches_loaded`
                // re-syncs the canonical value.
                self.thread_list.pinned_search_updated_at = active_id.map(|id| {
                    self.pinned_searches
                        .iter()
                        .find(|p| p.id == id)
                        .map_or_else(|| chrono::Utc::now().timestamp(), |ps| ps.updated_at)
                });
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

    /// Handle smart folder selection from the sidebar - fill the search bar
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

        // Static operators - resolve immediately
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

        // Dynamic operators - query DB asynchronously
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

    /// Handle typeahead selection - insert the value into the query.
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

impl ReadyApp {
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

                // Re-sync the staleness indicator if a pinned search is
                // active - the list reload is the authoritative source
                // for `updated_at` after fresh create / refresh.
                if let Some(active_id) = self.sidebar.active_pinned_search {
                    self.thread_list.pinned_search_updated_at = self
                        .pinned_searches
                        .iter()
                        .find(|p| p.id == active_id)
                        .map(|ps| ps.updated_at);
                }

                let db = Arc::clone(&self.db);
                Task::perform(
                    async move { db.get_recent_search_queries(10).await },
                    Message::SearchHistoryLoaded,
                )
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
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!(
                "pinned_search.delete: no ServiceClient yet; \
                 dismiss not persisted (next reload reconciles)"
            );
            return Task::none();
        };
        Task::perform(
            async move {
                let result = client.delete_pinned_search(id).await.map_err(|e| e.to_string());
                (id, result)
            },
            |(id, result)| Message::PinnedSearchDeleteAck(id, result),
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

    #[allow(clippy::needless_pass_by_value)]
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

    /// Clear pinned search context on navigate-away.
    pub(crate) fn clear_pinned_search_context(&mut self) {
        self.sidebar.active_pinned_search = None;
        self.editing_pinned_search = None;
        self.thread_list.pinned_search_updated_at = None;
    }

    /// Clear all search-related state without restoring pre-search threads.
    pub(crate) fn clear_search_state(&mut self) {
        self.search_query.reset(String::new());
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        let _ = self.search_generation.next();
        self.thread_list.mode = ThreadListMode::Scope;
        self.was_in_scope_view = false;
    }

    /// Restore the thread list to folder view after clearing search.
    ///
    /// Instead of restoring a stale clone of the pre-search thread list,
    /// this reloads from the database using the current navigation state.
    /// This avoids the O(n) clone on every search entry and ensures the
    /// thread list reflects any changes that happened during the search.
    pub(crate) fn restore_folder_view(&mut self) -> Task<Message> {
        self.thread_list.mode = ThreadListMode::Scope;
        self.search_query.reset(String::new());
        self.thread_list.search_query.clear();
        self.clear_pinned_search_context();
        self.clear_thread_selection();
        if self.was_in_scope_view {
            self.was_in_scope_view = false;
            let token = self.nav_generation.next();
            return self.load_threads_for_current_view(token);
        }
        Task::none()
    }
}

/// Execute search off the main thread via spawn_blocking.
///
/// Uses the provided SearchReadState (initialized once at boot) for Tantivy.
/// Falls back to SQL-only if no SearchReadState is available.
pub(crate) async fn execute_search(
    db: Arc<db::Db>,
    search_state: Option<Arc<rtsk::search::SearchReadState>>,
    body_store: Option<rtsk::body_store::BodyStoreReadState>,
    query: String,
    scope: rtsk::db::types::AccountScope,
) -> Result<Vec<Thread>, String> {
    db.with_read(move |conn| {
        // M2 fix: pass body_store so per-message attribution can score body
        // matches. Without it, body+attachment co-matches skewed toward
        // attachment attribution and the documented "matched in body +
        // also_matched: [Attachment]" outcome never appeared. Optional
        // because body_store init can fail at boot; the attribution falls
        // back to subject/from/attachments-only when None.
        let results = rtsk::search_pipeline::search(
            &query,
            search_state.as_deref(),
            conn,
            &scope,
            body_store.as_ref(),
        )?;
        let results = match results {
            rtsk::search_pipeline::SearchResults::FullIndex(results) => results,
            rtsk::search_pipeline::SearchResults::Degraded(results) => {
                // Keep the match at the UI boundary so degraded search can
                // grow a banner here without changing the search API again.
                results
            }
        };
        let mut threads = results
            .into_iter()
            .map(Thread::from_search_result)
            .filter(|thread| thread_matches_scope(thread, &scope))
            .collect::<Vec<_>>();
        crate::helpers::apply_thread_decorations(conn, &mut threads)?;
        Ok(threads)
    })
    .await
}

fn thread_matches_scope(thread: &Thread, scope: &AccountScope) -> bool {
    match scope {
        AccountScope::All => true,
        AccountScope::Single(id) => thread.account_id == *id,
        AccountScope::Multiple(ids) => ids.contains(&thread.account_id),
    }
}

fn pinned_search_scope_name(app: &ReadyApp, ps: &db::PinnedSearch) -> String {
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

impl ReadyApp {
    pub(crate) fn handle_refresh_pinned_search(&mut self, id: i64) -> Task<Message> {
        if !self.pinned_searches.iter().any(|ps| ps.id == id) {
            return Task::none();
        }

        let intent = SearchIntent::PinnedRefresh { id };
        let resolved = resolve_search_intent(intent, &self.current_ui_search_context());
        self.dispatch_resolved_search(resolved)
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
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!(
                "smart_folder.create: no ServiceClient yet; \
                 smart folder not saved (next reload reconciles)"
            );
            return Task::none();
        };
        Task::perform(
            async move { client.create_smart_folder(name, query).await.map_err(|e| e.to_string()) },
            Message::SmartFolderCreateAck,
        )
    }

    pub(crate) fn handle_smart_folder_saved(
        &mut self,
        result: Result<String, String>,
    ) -> Task<Message> {
        match result {
            Ok(_id) => {
                log::info!("Smart folder saved");

                // Graduation: the smart folder was created from a pinned
                // search, so the pinned search has been promoted and the
                // row should not linger. Dismiss it (the ack handler
                // clears local state and restores the prior folder view)
                // and reload the navigation state so the new smart
                // folder appears in the sidebar.
                let dismiss_task = if let Some(id) = self.editing_pinned_search.take() {
                    self.handle_dismiss_pinned_search(id)
                } else {
                    Task::none()
                };

                let token = self.nav_generation.next();
                let nav_task = self.fire_navigation_load(token);
                Task::batch([dismiss_task, nav_task])
            }
            Err(e) => {
                log::error!("Save smart folder error: {e}");
                Task::none()
            }
        }
    }
}
