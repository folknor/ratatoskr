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
            // Service{ChildSpawned,BootReady} re-fire post-handshake when a
            // respawn cycles (scope item 14). The App's notification
            // subscription is still bound to the same Arc<NotificationQueue>
            // and the schema-version sanity check already ran in
            // handle_crash, so there's nothing for ReadyApp to do here -
            // log at debug as a respawn breadcrumb.
            Message::ServiceChildSpawned(_) | Message::ServiceBootReady(_) => {
                log::debug!("ReadyApp observed post-handshake respawn event");
                Task::none()
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
                }
            }
            Message::ActionDispatched { plan_id, result } => {
                self.handle_action_dispatched(plan_id, result)
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
            Message::SyncCurrentFolder => self.sync_all_accounts(),
            Message::SyncTick => {
                let sync_task = self.sync_all_accounts();
                let pending_task = self.process_pending_ops();
                let gal_task = self.refresh_gal_caches();
                let cal_task = self.sync_calendars();
                Task::batch([sync_task, pending_task, gal_task, cal_task])
            }
            Message::SyncComplete(account_id, result) => {
                // Free the handle so the next tick can re-dispatch.
                self.sync_handles.remove(&account_id);
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
            Message::PinnedSearchDismissed(id, result) => {
                self.handle_pinned_search_dismissed(id, result)
            }
            Message::PinnedSearchPersisted(completion, result) => {
                self.handle_pinned_search_persisted(completion, result)
            }
            Message::PinnedSearchesExpired(result) => self.handle_pinned_searches_expired(result),
            Message::RefreshPinnedSearch(id) => self.handle_refresh_pinned_search(id),
            Message::ExpiryTick => self.handle_expiry_tick(),
            Message::SearchHere(prefix) => self.handle_search_here(prefix),
            Message::SaveAsSmartFolder(name) => self.handle_save_as_smart_folder(name),
            Message::SmartFolderSaved(result) => self.handle_smart_folder_saved(result),

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
                    return self.dispatch_undo(entry);
                }
                Task::none()
            }
            Message::UndoCompleted { desc, ref outcomes } => {
                if outcomes.is_empty() {
                    return Task::none();
                }
                let all_failed = outcomes.iter().all(rtsk::actions::ActionOutcome::is_failed);
                let any_failed = outcomes.iter().any(rtsk::actions::ActionOutcome::is_failed);
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
            Message::GalRefreshTick => {
                // Refresh GAL cache for all connected accounts.
                // Currently a placeholder - the actual directory API calls
                // (Graph /users, Google Directory API) require provider
                // clients. When the sync orchestrator provides account-level
                // clients, this dispatches cache_gal_entries() per account.
                log::debug!("GAL refresh tick (directory fetch not yet wired to provider clients)");
                Task::none()
            }
            Message::GalCacheRefreshed(result) => {
                match result {
                    Ok(count) => log::info!("GAL cache refreshed: {count} entries"),
                    Err(e) => log::warn!("GAL cache refresh failed: {e}"),
                }
                Task::none()
            }
            Message::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers;
                Task::none()
            }
            Message::AutoReplyChecked(active) => {
                self.status_bar.set_auto_reply_active(active);
                Task::none()
            }
        }
    }
}
