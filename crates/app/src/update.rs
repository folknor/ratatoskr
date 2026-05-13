use crate::app::ReadyApp;
use crate::component::Component;
use crate::message::Message;
use crate::pop_out::PopOutWindow;
use crate::pop_out::compose::ComposeMode;
use crate::ui;
use crate::ui::add_account::AddAccountWizard;
use crate::ui::calendar::CalendarMessage;
use crate::ui::layout::RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH;
use crate::{app::AppMode, handlers};
use iced::Task;
use std::sync::Arc;

impl ReadyApp {
    /// Central message dispatch. Each arm should be a ONE-LINE delegation
    /// to a handler method in `handlers/*.rs`. Do not inline logic here.
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // Component delegation
            Message::Sidebar(msg) => self.handle_sidebar(msg),
            Message::ThreadList(msg) => self.handle_thread_list(msg),
            Message::ReadingPane(msg) => self.handle_reading_pane(msg),
            Message::Settings(msg) => self.handle_settings(msg),
            Message::StatusBar(msg) => self.handle_status_bar(msg),

            // Appearance
            Message::AppearanceChanged(mode) => {
                self.mode = mode;
                Task::none()
            }

            // Data loading with generation guards
            Message::AccountsLoaded(g, _) if !self.nav_generation.is_current(g) => Task::none(),
            Message::AccountsLoaded(_, Ok(accounts)) => {
                log::info!("Loaded {} accounts", accounts.len());
                self.handle_accounts_loaded(accounts)
            }
            Message::AccountsLoaded(_, Err(e)) => {
                log::error!("AccountsLoaded error: {e}");
                self.status = format!("Error: {e}");
                Task::none()
            }
            Message::NavigationLoaded(g, _) if !self.nav_generation.is_current(g) => Task::none(),
            Message::NavigationLoaded(_, Ok(nav_state)) => {
                self.sidebar.nav_state = Some(nav_state);
                Task::none()
            }
            Message::NavigationLoaded(_, Err(e)) => {
                log::error!("Navigation load error: {e}");
                self.status = format!("Navigation error: {e}");
                Task::none()
            }
            Message::ThreadsLoaded(g, _) if !self.nav_generation.is_current(g) => Task::none(),
            Message::ThreadsLoaded(_, Ok(threads)) => {
                log::info!("Loaded {} threads", threads.len());
                self.status = format!("{} threads", threads.len());
                self.thread_list.set_threads(threads);
                Task::none()
            }
            Message::ThreadsLoaded(_, Err(e)) => {
                log::error!("ThreadsLoaded error: {e}");
                self.status = format!("Threads error: {e}");
                Task::none()
            }

            // Divider drag
            Message::DividerDragStart(divider) => {
                self.dragging = Some(divider);
                Task::none()
            }
            Message::DividerDragMove(point) => self.handle_divider_drag(point),
            Message::DividerDragEnd => {
                self.dragging = None;
                Task::none()
            }
            Message::DividerHover(divider) => {
                self.hovered_divider = Some(divider);
                Task::none()
            }
            Message::DividerUnhover => {
                self.hovered_divider = None;
                Task::none()
            }

            // Settings and UI toggles
            Message::ToggleSettings => {
                if self.show_settings {
                    self.close_settings();
                } else {
                    self.open_settings(crate::ui::settings::Tab::General);
                }
                Task::none()
            }
            Message::ToggleRightSidebar => {
                self.right_sidebar_open = !self.right_sidebar_open;
                Task::none()
            }
            Message::SettingsCheckFocus => {
                if !self.show_settings {
                    return Task::none();
                }
                iced::advanced::widget::operate(
                    crate::ui::settings::focus_query::find_focused_filter(),
                )
                .map(|maybe| {
                    Message::Settings(
                        crate::ui::settings::SettingsMessage::FilterFocusUpdated(maybe),
                    )
                })
            }
            Message::SetDateDisplay(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }

            // Window management
            Message::WindowResized(id, size) => {
                if id == self.main_window_id {
                    self.window.set_size(size);
                    if size.width < RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH && self.right_sidebar_open {
                        self.right_sidebar_open = false;
                    }
                    // Clamp panel widths so minimums are respected after
                    // the window shrinks.
                    self.clamp_panel_widths();
                } else {
                    match self.pop_out_windows.get_mut(&id) {
                        Some(PopOutWindow::MessageView(state)) => {
                            state.width = size.width;
                            state.height = size.height;
                        }
                        Some(PopOutWindow::Compose(state)) => {
                            state.width = size.width;
                            state.height = size.height;
                        }
                        Some(PopOutWindow::Calendar(geom)) => {
                            geom.width = size.width;
                            geom.height = size.height;
                        }
                        None => {}
                    }
                }
                Task::none()
            }
            Message::WindowMoved(id, point) => {
                if id == self.main_window_id {
                    self.window.set_position(point);
                } else {
                    match self.pop_out_windows.get_mut(&id) {
                        Some(PopOutWindow::MessageView(state)) => {
                            state.x = Some(point.x);
                            state.y = Some(point.y);
                        }
                        Some(PopOutWindow::Compose(state)) => {
                            state.x = Some(point.x);
                            state.y = Some(point.y);
                        }
                        Some(PopOutWindow::Calendar(geom)) => {
                            geom.x = Some(point.x);
                            geom.y = Some(point.y);
                        }
                        None => {}
                    }
                }
                Task::none()
            }
            Message::WindowCloseRequested(id) => self.handle_window_close(id),

            // Compose
            Message::Compose => self.open_compose_window(ComposeMode::New),
            Message::Noop => Task::none(),
            Message::AttachmentSaveFolderRemembered(key, folder) => {
                self.attachment_last_folders.insert(key, folder);
                Task::none()
            }
            // ServiceChildSpawned re-fires post-handshake on every
            // respawn cycle. The notification subscription is bound to
            // the same Arc<NotificationQueue> across respawns, so this
            // is informational - log a breadcrumb and move on.
            Message::ServiceChildSpawned(_) => {
                log::debug!("ReadyApp observed post-handshake respawn event");
                Task::none()
            }
            // ServiceBootReady re-fires after every successful respawn
            // handshake (Phase 1.5 scope item 14). Phase 2 scope item 11
            // / 18d: this is the trigger for `action.job_status`
            // reconciliation - any plan stuck in `AckUnknown` since the
            // last incarnation needs to resolve before the UI dispatches
            // any new action against the same accounts.
            Message::ServiceBootReady(_response) => {
                log::debug!("ReadyApp observed post-handshake respawn event");
                // Phase 7-6: catch-up kick for cached-but-unindexed
                // attachments left over from a prior Service crash
                // mid-extraction. Idempotent on repeat (the SELECT
                // returns 0 rows after the first kick drains the
                // backlog).
                Task::batch(vec![
                    self.kickoff_post_respawn_reconcile(),
                    self.kick_extract_backfill(),
                ])
            }
            // ServiceBootFailed reaching ReadyApp means a respawn attempt
            // failed (e.g., the new Service exited with a deterministic
            // BootExitCode, or the new boot.ready handshake itself errored).
            // Phase 1.5 surfaces this as a fatal error and exits cleanly,
            // identical to the boot-time path; Phase 8's UI status indicator
            // will replace the iced::exit() with an in-app banner.
            Message::ServiceBootFailed(reason) => {
                let _ = crate::service_client::surface_terminal_failure(&reason);
                iced::exit()
            }
            // Phase 8-1: store the latest health and let the status-bar
            // component render. PersistentlyFailing is an authoritative
            // "Service is gone, take action" surface; the rest are
            // transient transition markers (Booting / Respawning) that
            // resolve back to Healthy on the next successful BootReady.
            Message::ServiceHealthChanged(health) => {
                log::info!("service health: {health:?}");
                self.service_health = health;
                Task::none()
            }
            // Phase 8-1: async store-init completion. UI surfaces that
            // depend on a store render "loading..." until these arms
            // populate the corresponding `Option<...>` field.
            Message::BodyStoreReady(Ok(store)) => {
                self.body_store = Some(store);
                Task::none()
            }
            Message::BodyStoreReady(Err(e)) => {
                log::error!("body store async init failed: {e}");
                Task::none()
            }
            Message::InlineImageStoreReady(Ok(store)) => {
                self.inline_image_store = Some(store);
                Task::none()
            }
            Message::InlineImageStoreReady(Err(e)) => {
                log::error!("inline image store async init failed: {e}");
                Task::none()
            }
            Message::SearchStateReady(Ok(state)) => {
                self.search_state = Some(state);
                Task::none()
            }
            Message::SearchStateReady(Err(e)) => {
                log::error!("search state async init failed: {e}");
                Task::none()
            }
            Message::ServiceNotification(notification) => {
                // Drop notifications from a dying-but-still-flushing reader
                // after a respawn (item 15 of phase-1.5-plan.md). The
                // generation check applies to every variant; per-variant
                // dispatch happens after the gen-filter.
                let current_gen = self
                    .service_client
                    .as_ref()
                    .map(|c| c.current_generation())
                    .unwrap_or(0);
                if !crate::service_client::notification_should_dispatch(
                    &notification,
                    current_gen,
                ) {
                    return Task::none();
                }
                match notification {
                    service_api::Notification::OperationOutcome(outcome) => {
                        self.handle_notification_operation_outcome(outcome)
                    }
                    service_api::Notification::ActionCompleted(completion) => {
                        self.handle_notification_action_completed(&completion)
                    }
                    service_api::Notification::SyncProgress(_)
                    | service_api::Notification::BootProgress(_) => {
                        // BootProgress is uninteresting post-Ready;
                        // SyncProgress is a Phase 3 surface (no UI
                        // consumer wired yet).
                        log::debug!("Service notification: {}", notification.method_name());
                        Task::none()
                    }
                    service_api::Notification::SyncCompleted(_) => {
                        // Phase 3 task 14: SyncCompleted is consumed
                        // inside `ServiceClient::route_sync_completed`
                        // before the reader task enqueues. Reaching
                        // this arm means the routing was bypassed
                        // (test path that bumps the queue directly,
                        // or a stale notification past respawn) - drop
                        // with a debug log.
                        log::debug!(
                            "SyncCompleted reached update arm; routing already consumed it",
                        );
                        Task::none()
                    }
                    service_api::Notification::IndexCommitted(_) => {
                        // Phase 3 task 17: stamp the pending reload
                        // deadline; the 200 ms `ReaderReloadTick`
                        // handler calls `reader.reload()` once the
                        // stamp has aged past one tick. Debounces a
                        // commit storm under heavy initial sync.
                        self.pending_reader_reload = Some(std::time::Instant::now());
                        Task::none()
                    }
                    service_api::Notification::PushEvent(event) => {
                        // Phase 4 task 8 + review-pass: stamp the
                        // most-recent-push timestamp and fire a
                        // rate-limited "New mail in <email>"
                        // confirmation. The Service has already kicked
                        // sync by the time we get here; this arm is
                        // purely UI.
                        let label = self.email_for_account(&event.account_id);
                        self.status_bar
                            .record_push_event(event.account_id, &label);
                        Task::none()
                    }
                    service_api::Notification::CalendarRunCompleted(_) => {
                        // Phase 5: CalendarRunCompleted is consumed inside
                        // ServiceClient (per-run_id awaiters) before the
                        // reader task enqueues - mirrors SyncCompleted's
                        // routing. Reaching this arm means the routing was
                        // bypassed (test path or a stale notification past
                        // respawn); drop with a debug log.
                        log::debug!(
                            "CalendarRunCompleted reached update arm; routing already consumed it",
                        );
                        Task::none()
                    }
                    service_api::Notification::CalendarChanged(_) => {
                        // Phase 5 task 11: stamp the pending reload deadline.
                        // The 250 ms `CalendarReloadTick` handler calls
                        // `reload_calendar_events()` once the stamp has aged
                        // past one tick. Debounces a kick batch's worth of
                        // CalendarChanged notifications into a single reload.
                        self.pending_calendar_reload = Some(std::time::Instant::now());
                        Task::none()
                    }
                    service_api::Notification::CalendarOperationOutcome(_) => {
                        // Phase 6c-9: per-op outcomes are advisory in
                        // the 1:1-plan world. The completion frame
                        // (CalendarActionCompleted) is what the UI
                        // awaits via `pending_calendar_actions`.
                        // Phase 6d may grow a per-op consumer if
                        // N-op plans land.
                        Task::none()
                    }
                    service_api::Notification::CalendarActionCompleted(_) => {
                        // Phase 6c-9: consumed inside
                        // `ServiceClient::route_calendar_action_completed`
                        // before the reader task enqueues. Reaching this
                        // arm means routing was bypassed (test path or a
                        // stale notification past respawn) - drop with
                        // a debug log.
                        log::debug!(
                            "CalendarActionCompleted reached update arm; routing already \
                             consumed it",
                        );
                        Task::none()
                    }
                    service_api::Notification::ExtractProgress(p) => {
                        log::debug!(
                            "extract progress: remaining={}, indexed_in_session={}",
                            p.remaining, p.indexed_in_session,
                        );
                        Task::none()
                    }
                    service_api::Notification::ExtractCompleted(c) => {
                        log::info!(
                            "extract completed: indexed={}, skipped={}, failed={}",
                            c.indexed, c.skipped, c.failed,
                        );
                        Task::none()
                    }
                    service_api::Notification::PrefetchProgress(p) => {
                        log::debug!(
                            "prefetch progress: remaining={}, fetched_in_session={}",
                            p.remaining, p.fetched_in_session,
                        );
                        Task::none()
                    }
                    service_api::Notification::PrefetchCompleted(c) => {
                        log::info!(
                            "prefetch completed: fetched={}, skipped={}, failed={}",
                            c.fetched, c.skipped, c.failed,
                        );
                        Task::none()
                    }
                    service_api::Notification::IndexRebuildProgress(p) => {
                        log::debug!(
                            "index rebuild {}: {}/{}",
                            p.rebuild_id, p.processed, p.total,
                        );
                        // Phase 8-4: store the latest progress for the
                        // status-bar component. The component renders a
                        // small banner when this is `Some`; cleared on
                        // IndexRebuildCompleted.
                        self.index_rebuild_progress = Some(crate::app::RebuildProgressState {
                            rebuild_id: p.rebuild_id,
                            processed:  p.processed,
                            total:      p.total,
                        });
                        Task::none()
                    }
                    service_api::Notification::IndexRebuildCompleted(c) => {
                        log::info!("index rebuild {} completed", c.rebuild_id);
                        // Phase 8-4: clear the progress state and
                        // re-init `SearchReadState` so the new index
                        // is reachable in-session. Without the
                        // re-init the UI keeps the stale reader
                        // handle until the next launch.
                        self.index_rebuild_progress = None;
                        let data_dir = match crate::APP_DATA_DIR.get() {
                            Some(d) => d.clone(),
                            None => return Task::none(),
                        };
                        Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || {
                                    rtsk::search::SearchReadState::init(&data_dir).map(Arc::new)
                                })
                                .await
                                .map_err(|e| format!("search re-init join: {e}"))?
                            },
                            Message::SearchStateReady,
                        )
                    }
                }
            }
            Message::ActionDispatched { plan_id, outcome } => {
                self.handle_action_dispatched(plan_id, outcome)
            }
            Message::JobStatusResolved { plan_id, result } => {
                self.handle_job_status_resolved(plan_id, result)
            }
            Message::ServiceShutdownComplete(result) => {
                if let Err(error) = result {
                    log::warn!("Service shutdown failed: {error}");
                }
                iced::exit()
            }

            // Command system
            Message::KeyEvent(msg) => self.handle_key_event(msg),
            Message::ExecuteCommand(id) => self.handle_execute_command(id),
            Message::ExecuteParameterized(id, args) => self.handle_execute_parameterized(id, args),
            Message::NavigateTo(target) => self.handle_navigate_to(target),
            Message::Escape => {
                // Route calendar Escape through the calendar's own handlers
                // so workflow-aware behavior runs - importantly, Escape from
                // the editor's ConfirmDiscard returns to the editor with
                // the draft intact instead of nuking workflow back to Idle.
                if self.calendar.active_popover.is_some() {
                    return self
                        .update(Message::Calendar(Box::new(
                            crate::ui::calendar::CalendarMessage::ClosePopover,
                        )));
                }
                if self.calendar.active_modal.is_some() {
                    return self
                        .update(Message::Calendar(Box::new(
                            crate::ui::calendar::CalendarMessage::CloseModal,
                        )));
                }
                if !matches!(
                    self.calendar.workflow,
                    crate::ui::calendar::CalendarWorkflow::Idle
                ) {
                    // Workflow is non-Idle but no surface is showing -
                    // fall back to the blunt reset rather than stranding
                    // an unreachable workflow state.
                    self.calendar.workflow = crate::ui::calendar::CalendarWorkflow::Idle;
                    self.calendar.sync_surfaces();
                    return Task::none();
                }
                if self.show_settings {
                    self.close_settings();
                    return Task::none();
                }
                if !self.search_query.text().is_empty()
                    || self.sidebar.active_pinned_search.is_some()
                {
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    return self.update(Message::SearchClear);
                }
                Task::none()
            }
            Message::EmailAction(action) => self.handle_email_action(action),
            Message::ActionCompleted {
                ref plan,
                ref outcomes,
            } => self.handle_action_completed(plan, outcomes),
            Message::SendCompleted {
                window_id,
                ref outcome,
            } => self.handle_send_completed(window_id, outcome),
            Message::ComposeAction(ref action) => self.handle_compose_action(action),
            Message::TaskAction(_action) => Task::none(),
            Message::SetTheme(theme) => {
                self.settings.theme = theme;
                Task::none()
            }
            Message::ToggleSidebar => Task::none(),
            Message::FocusSearch => self.update(Message::FocusSearchBar),
            Message::ShowHelp => Task::none(),
            Message::SyncCurrentFolder => {
                // Manual "sync now" bypasses staleness gates: explicit
                // request fan-out for both email and calendar.
                Task::batch([self.sync_all_accounts(), self.calendar_sync_all_accounts()])
            }
            Message::SyncTick => {
                // Phase 5 task 10 + Phase 6a + 6b: SyncTick collapses to
                // five notifications + one request fan-out. Calendar,
                // GAL, pinned-search expiry, and attachment-cache
                // eviction all relocated Service-side; the UI's only
                // role is to fire the cadence. Service-side staleness
                // gates (calendar: 1h per-account last_completed; GAL:
                // 24h cache check; pinned-search: 14-day creation age;
                // attachment eviction: 200 MB per-kick reclaim cap)
                // keep the actual work bounded.
                let sync_task = self.sync_all_accounts();
                let pending_task = self.process_pending_ops();
                let gal_task = self.kick_gal_refresh();
                let cal_task = self.kick_calendar_sync();
                let pinned_task = self.kick_pinned_search_expire();
                let attachment_task = self.kick_attachment_eviction();
                Task::batch([
                    sync_task,
                    pending_task,
                    gal_task,
                    cal_task,
                    pinned_task,
                    attachment_task,
                ])
            }
            Message::SyncComplete(account_id, result) => {
                // Phase 3 task 15: per-account "already-in-flight" gating
                // moved Service-side (`SyncRuntime` keys runs by
                // `SyncRunId`); no UI-side handle to free here.
                // If the account was deleted while sync was in-flight, drop
                // the result silently. The status-bar warning, navigation
                // reload, and chat-view refresh below all assume the account
                // still exists.
                if !self.sidebar.accounts.iter().any(|a| a.id == account_id) {
                    log::debug!(
                        "Dropping sync result for unknown/deleted account {account_id}"
                    );
                    return Task::none();
                }
                match result {
                    Err(ref e) => {
                        log::error!("Sync failed for {account_id}: {e}");
                        let lower = e.to_lowercase();
                        let is_auth_error = lower.contains("401")
                            || lower.contains("unauthorized")
                            || lower.contains("token")
                            || lower.contains("auth")
                            || lower.contains("expired")
                            || lower.contains("invalid_grant")
                            || lower.contains("refresh");
                        let email = self.email_for_account(&account_id);
                        if is_auth_error {
                            self.status_bar.set_warning(ui::status_bar::AccountWarning {
                                account_id: account_id.clone(),
                                email,
                                kind: ui::status_bar::WarningKind::TokenExpiry,
                            });
                        } else {
                            self.status_bar.set_warning(ui::status_bar::AccountWarning {
                                account_id: account_id.clone(),
                                email,
                                kind: ui::status_bar::WarningKind::ConnectionFailure {
                                    message: e.clone(),
                                },
                            });
                        }
                    }
                    Ok(()) => {
                        // Sync succeeded - clear any previous warning for this account
                        self.status_bar.clear_warning(&account_id);
                    }
                }
                // Reload navigation + threads (or chat timeline) to reflect sync changes
                if let Some(email) = self.active_chat.clone() {
                    return self.enter_chat_view(&email);
                }
                let _ = self.nav_generation.next();
                let nav_task = self.load_navigation_and_threads();
                let auto_reply_task = self.check_auto_reply_status();
                Task::batch([nav_task, auto_reply_task])
            }
            Message::SetReadingPanePosition(_pos) => Task::none(),
            Message::Palette(msg) => self.handle_palette(msg),

            // Search - delegated to handlers/search.rs
            Message::SearchQueryChanged(query) => self.handle_search_query_changed(query),
            Message::SearchExecute => self.handle_search_execute(),
            Message::SearchCompleted(result) => self.handle_search_completed(result),
            Message::SearchClear => self.handle_search_clear(),
            Message::FocusSearchBar => self.handle_focus_search_bar(),
            Message::SearchBlur => {
                self.thread_list.typeahead.visible = false;
                // Focus a non-existent widget to remove focus from the search bar.
                // iced ignores focus operations on unknown IDs, but the act of
                // issuing any focus operation clears the current focus.
                iced::widget::operation::focus::<Message>("blur-sink".to_string())
            }

            Message::SearchHistoryLoaded(Ok(queries)) => {
                self.search_history = queries;
                Task::none()
            }
            Message::SearchHistoryLoaded(Err(_)) => Task::none(),

            // Pinned searches - delegated to handlers/search.rs
            Message::PinnedSearchesLoaded(result) => self.handle_pinned_searches_loaded(result),
            Message::SelectPinnedSearch(id) => self.handle_select_pinned_search(id),
            Message::DismissPinnedSearch(id) => self.handle_dismiss_pinned_search(id),
            Message::PinnedSearchDeleteAck(id, result) => {
                self.handle_pinned_search_dismissed(id, result)
            }
            Message::PinnedSearchCreateOrUpdateAck(completion, result)
            | Message::PinnedSearchUpdateAck(completion, result) => {
                // Both IPCs share the post-persist UI behavior (reload
                // sidebar, surface error in status). Wire-level
                // distinction is preserved via the variant; behavioural
                // divergence is a one-line edit if it ever lands.
                self.handle_pinned_search_persisted(completion, result)
            }
            Message::PinnedSearchDeleteAllAck(result) => {
                if let Err(e) = result {
                    log::error!("Failed to clear pinned searches: {e}");
                }
                Task::none()
            }
            Message::RefreshPinnedSearch(id) => self.handle_refresh_pinned_search(id),
            Message::SearchHere(prefix) => self.handle_search_here(prefix),
            Message::SaveAsSmartFolder(name) => self.handle_save_as_smart_folder(name),
            Message::SmartFolderCreateAck(result) => self.handle_smart_folder_saved(result),

            // Calendar - delegated to handlers/calendar.rs
            Message::Calendar(cal_msg) => self.handle_calendar(*cal_msg),
            Message::ToggleAppMode => {
                // If calendar is popped out, focus the pop-out instead of toggling
                if self.app_mode == AppMode::Mail
                    && let Some(win_id) = self.calendar_pop_out_id()
                {
                    return iced::window::gain_focus(win_id);
                }
                let target = match self.app_mode {
                    AppMode::Mail => AppMode::Calendar,
                    AppMode::Calendar => AppMode::Mail,
                };
                self.update(Message::SetAppMode(target))
            }
            Message::SetAppMode(mode) => {
                // If switching to calendar while it's popped out, focus the pop-out
                if mode == AppMode::Calendar
                    && let Some(win_id) = self.calendar_pop_out_id()
                {
                    return iced::window::gain_focus(win_id);
                }
                if self.app_mode == mode {
                    return Task::none();
                }
                self.app_mode = mode;
                if self.app_mode == AppMode::Calendar {
                    return self.reload_calendar_events();
                }
                Task::none()
            }
            Message::SetCalendarView(view) => {
                // Route to pop-out if calendar is popped out
                if let Some(win_id) = self.calendar_pop_out_id() {
                    return iced::window::gain_focus(win_id);
                }
                if self.app_mode != AppMode::Calendar {
                    self.app_mode = AppMode::Calendar;
                }
                self.update(Message::Calendar(Box::new(CalendarMessage::SetView(view))))
            }
            Message::CalendarToday => {
                // Route to pop-out if calendar is popped out
                if let Some(win_id) = self.calendar_pop_out_id() {
                    return iced::window::gain_focus(win_id);
                }
                self.update(Message::Calendar(Box::new(CalendarMessage::Today)))
            }
            Message::CalendarSyncComplete => self.reload_calendar_events(),

            // Account management
            Message::AddAccount(msg) => self.handle_add_account(msg),
            Message::AccountDeleted(Ok(())) | Message::AccountUpdated(Ok(())) => {
                // Reload accounts after delete or update
                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation.next();
                Task::perform(
                    async move { (load_gen, crate::helpers::load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
            }
            Message::AccountDeleted(Err(e)) => {
                log::error!("Failed to delete account: {e}");
                Task::none()
            }
            Message::AccountUpdated(Err(e)) => {
                log::error!("Failed to update account: {e}");
                Task::none()
            }
            Message::OpenAddAccount => {
                let used_colors = self
                    .sidebar
                    .accounts
                    .iter()
                    .filter_map(|a| a.account_color.clone())
                    .collect();
                self.add_account_wizard = Some(AddAccountWizard::new_add_account(
                    used_colors,
                    Arc::clone(&self.db),
                    self.service_client.clone(),
                ));
                Task::none()
            }
            Message::ReloadSignatures => {
                handlers::signatures::load_signatures_async(&self.db).map(Message::SignatureOp)
            }
            Message::SignatureOp(result) => self.handle_signature_op(result),

            // Pop-out windows - delegated to handlers/pop_out.rs
            Message::PopOut(window_id, pop_out_msg) => {
                self.handle_pop_out_message(window_id, pop_out_msg)
            }
            Message::OpenMessageView(message_index) => self.open_message_view_window(message_index),
            Message::ComposeDraftTick => self.auto_save_compose_drafts(),
            Message::LocalDraftLoaded(Ok(Some(draft))) => {
                let state = crate::pop_out::compose::ComposeState::from_local_draft(
                    &self.sidebar.accounts,
                    &draft,
                );
                self.open_compose_window_with_state(
                    state,
                    crate::pop_out::compose::ComposeMode::New,
                )
            }
            Message::LocalDraftLoaded(Ok(None)) => {
                log::warn!("Local draft not found in DB");
                Task::none()
            }
            Message::LocalDraftLoaded(Err(e)) => {
                log::error!("Failed to load local draft: {e}");
                Task::none()
            }
            Message::RestoredComposeLoaded {
                window_id,
                width,
                height,
                x,
                y,
                result,
            } => self.handle_restored_compose_loaded(window_id, width, height, x, y, result),

            // Thread detail via core (replaces separate messages/attachments loads)
            Message::ThreadDetailLoaded(g, _) if !self.thread_generation.is_current(g) => {
                Task::none()
            }
            Message::ThreadDetailLoaded(_, Ok(detail)) => {
                self.reading_pane.load_thread_detail(detail);
                Task::none()
            }
            Message::ThreadDetailLoaded(_, Err(e)) => {
                log::error!("ThreadDetailLoaded error: {e}");
                self.status = format!("Thread detail error: {e}");
                Task::none()
            }

            // Chat timeline
            Message::ChatTimeline(msg) => {
                if let Some(ref mut timeline) = self.chat_timeline {
                    let (task, event) = timeline.update(msg);
                    let task = task.map(Message::ChatTimeline);
                    if let Some(event) = event {
                        return Task::batch([task, self.handle_chat_timeline_event(event)]);
                    }
                    return task;
                }
                Task::none()
            }
            Message::ChatTimelineLoaded(g, _) if !self.chat_generation.is_current(g) => {
                Task::none()
            }
            Message::ChatTimelineLoaded(_, Ok(messages)) => {
                self.handle_chat_timeline_loaded(messages)
            }
            Message::ChatTimelineLoaded(_, Err(e)) => {
                log::error!("ChatTimelineLoaded error: {e}");
                if let Some(ref mut tl) = self.chat_timeline {
                    tl.loading = false;
                }
                Task::none()
            }
            Message::ChatOlderLoaded(ref email, Ok(ref messages))
                if self
                    .chat_timeline
                    .as_ref()
                    .is_some_and(|t| t.contact_email == *email) =>
            {
                let msgs = messages.clone();
                self.handle_chat_older_loaded(msgs)
            }
            Message::ChatOlderLoaded(_, Ok(_)) => Task::none(), // stale - different chat
            Message::ChatOlderLoaded(_, Err(e)) => {
                log::error!("ChatOlderLoaded error: {e}");
                Task::none()
            }
            Message::ChatReadMarked => {
                let token = self.chat_list_generation.next();
                self.fire_chat_contacts_load(token)
            }
            Message::ChatContactsLoaded(g, _) if !self.chat_list_generation.is_current(g) => {
                Task::none()
            }
            Message::ChatContactsLoaded(_, Ok(contacts)) => {
                self.sidebar.chat_contacts = contacts;
                Task::none()
            }
            Message::ChatContactsLoaded(_, Err(e)) => {
                log::error!("ChatContactsLoaded error: {e}");
                Task::none()
            }

            // Clear all pinned searches
            Message::ClearAllPinnedSearches => self.handle_clear_all_pinned_searches(),

            // Sync progress pipeline
            Message::SyncProgress(event) => {
                self.handle_sync_event(event);
                Task::none()
            }
            Message::Undo => {
                if let Some(entry) = self.undo_stack.pop() {
                    return self.dispatch_undo(&entry);
                }
                Task::none()
            }
            Message::UndoCompleted { desc, ref outcomes } => {
                if outcomes.is_empty() {
                    return Task::none();
                }
                let all_failed = outcomes.iter().all(service::actions::ActionOutcome::is_failed);
                let any_failed = outcomes.iter().any(service::actions::ActionOutcome::is_failed);
                if all_failed {
                    self.status_bar
                        .show_confirmation(format!("\u{26A0} Undo failed: {desc}"));
                } else if any_failed {
                    self.status_bar.show_confirmation(
                        "\u{26A0} Undo partially failed \u{2014} some changes may revert"
                            .to_string(),
                    );
                } else {
                    self.status_bar.show_confirmation(format!("Undone: {desc}"));
                }
                {
                    let token = self.nav_generation.next();
                    Task::batch([
                        self.fire_navigation_load(token),
                        self.load_threads_for_current_view(token),
                    ])
                }
            }
            Message::SharedMailboxesLoaded(Ok(mailboxes)) => {
                self.sidebar.shared_mailboxes = mailboxes;
                Task::none()
            }
            Message::SharedMailboxesLoaded(Err(e)) => {
                log::warn!("Failed to load shared mailboxes: {e}");
                Task::none()
            }
            Message::PinnedPublicFoldersLoaded(Ok(pins)) => {
                self.sidebar.pinned_public_folders = pins;
                Task::none()
            }
            Message::PinnedPublicFoldersLoaded(Err(e)) => {
                log::warn!("Failed to load pinned public folders: {e}");
                Task::none()
            }
            Message::SnoozeTick => self.handle_snooze_tick(),
            Message::SnoozeResurfaceComplete(result) => {
                self.handle_snooze_resurface_complete(result)
            }
            Message::ExtractBackfillTick => self.kick_extract_backfill(),
            Message::RebuildSearchIndex => self.dispatch_rebuild_search_index(),
            Message::RebuildSearchIndexDispatched(result) => {
                match result {
                    Ok(id) => log::info!("rebuild dispatched: {id}"),
                    Err(e) => log::warn!("rebuild dispatch failed: {e}"),
                }
                Task::none()
            }
            Message::ReaderReloadTick => {
                // Phase 3 task 17: debounced reader reload. Skip when
                // there is no pending stamp (idle) or when the stamp
                // was set within the last tick window (a fresh
                // IndexCommitted just landed; the next tick will run
                // the reload).
                let Some(stamp) = self.pending_reader_reload else {
                    return Task::none();
                };
                if stamp.elapsed() < std::time::Duration::from_millis(200) {
                    return Task::none();
                }
                if let Some(reader) = self.search_state.as_ref()
                    && let Err(e) = reader.reload()
                {
                    log::warn!("SearchReadState::reload failed: {e}");
                }
                self.pending_reader_reload = None;
                Task::none()
            }
            Message::CalendarReloadTick => {
                // Phase 5 task 11: debounced calendar reload. Skip when
                // there's no pending stamp (idle) or when the stamp was
                // set within the last tick window (a fresh CalendarChanged
                // just landed; the next tick will run the reload).
                let Some(stamp) = self.pending_calendar_reload else {
                    return Task::none();
                };
                if stamp.elapsed() < std::time::Duration::from_millis(250) {
                    return Task::none();
                }
                self.pending_calendar_reload = None;
                self.reload_calendar_events()
            }
            Message::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers;
                Task::none()
            }
            Message::AutoReplyChecked(active) => {
                self.status_bar.set_auto_reply_active(active);
                Task::none()
            }
            Message::BootstrapSnapshotsLoaded(result) => {
                self.handle_bootstrap_snapshots_loaded(result);
                Task::none()
            }
        }
    }
}
