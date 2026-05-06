use crate::app::{AppMode, ReadyApp};
use crate::component::Component;
use crate::db;
use crate::handlers;
use crate::message::Message;
use crate::pop_out::{self, PopOutWindow, compose::ComposeMode};
use crate::ui;
use crate::ui::add_account::AddAccountMessage;
use crate::ui::reading_pane::{ReadingPaneEvent, ReadingPaneMessage};
use crate::ui::settings::{SettingsEvent, SettingsMessage};
use crate::ui::sidebar::{SidebarEvent, SidebarMessage};
use crate::ui::status_bar::{
    AccountWarning, StatusBarEvent, StatusBarMessage, SyncEvent, WarningKind,
};
use crate::ui::thread_list::{ThreadListEvent, ThreadListMessage};
use cmdk::CommandId;
use iced::Task;
use rtsk::scope::ViewScope;
use service_api::SettingValue;
use std::sync::Arc;

impl ReadyApp {
    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_sidebar(&mut self, msg: SidebarMessage) -> Task<Message> {
        let (task, event) = self.sidebar.update(msg);
        let mut tasks = vec![task.map(Message::Sidebar)];
        if let Some(evt) = event {
            tasks.push(self.handle_sidebar_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_sidebar_event(&mut self, event: SidebarEvent) -> Task<Message> {
        match event {
            SidebarEvent::AccountSelected(_idx) => {
                self.reset_view_state();
                self.load_navigation_and_threads()
            }
            SidebarEvent::AllAccountsSelected => {
                self.reset_view_state();
                self.load_navigation_and_threads()
            }
            SidebarEvent::SelectionChanged(_sel) => {
                self.reset_view_state();
                let token = self.nav_generation.next();
                self.load_threads_for_current_view(token)
            }
            SidebarEvent::Compose => self.update(Message::Compose),
            SidebarEvent::ToggleSettings => self.update(Message::ToggleSettings),
            SidebarEvent::PinnedSearchSelected(id) => self.update(Message::SelectPinnedSearch(id)),
            SidebarEvent::PinnedSearchDismissed(id) => {
                self.update(Message::DismissPinnedSearch(id))
            }
            SidebarEvent::ModeToggled => self.update(Message::ToggleAppMode),
            SidebarEvent::SearchHere { query_prefix } => {
                self.update(Message::SearchHere(query_prefix))
            }
            SidebarEvent::SmartFolderSelected { id, query } => {
                self.handle_smart_folder_selected(id, query)
            }
            SidebarEvent::PinnedSearchRefreshed(id) => {
                self.update(Message::RefreshPinnedSearch(id))
            }
            SidebarEvent::SharedMailboxSelected { .. } => {
                self.reset_view_state();
                self.load_navigation_and_threads()
            }
            SidebarEvent::PublicFolderSelected { .. } => {
                self.reset_view_state();
                self.load_navigation_and_threads()
            }
            SidebarEvent::ChatSelected(email) => self.enter_chat_view(&email),
        }
    }

    /// Full view-transition reset: clear search, pinned search, thread
    /// Reset view state: clear search, thread selection, chat, bump
    /// generations, and update thread list context.
    /// Call before loading threads/navigation for the new view.
    pub(crate) fn reset_view_state(&mut self) {
        self.clear_search_state();
        self.clear_pinned_search_context();
        self.active_chat = None;
        self.sidebar.active_chat = None;
        self.clear_thread_selection();
        self.chat_timeline = None;
        let _ = self.nav_generation.next();
        let _ = self.thread_generation.next();
        self.update_thread_list_context_from_sidebar();
    }

    /// Clear thread selection and reading pane together. Every code path that
    /// deselects threads must use this to prevent stale reading pane content.
    pub(crate) fn clear_thread_selection(&mut self) {
        self.thread_list.selected_thread = None;
        self.thread_list.clear_multi_select();
        self.reading_pane.set_thread(None);
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_thread_list(&mut self, msg: ThreadListMessage) -> Task<Message> {
        let (task, event) = self.thread_list.update(msg);
        let mut tasks = vec![task.map(Message::ThreadList)];
        if let Some(evt) = event {
            tasks.push(self.handle_thread_list_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_thread_list_event(&mut self, event: ThreadListEvent) -> Task<Message> {
        match event {
            ThreadListEvent::ThreadSelected(idx) => {
                // Check modifier keys for multi-select behavior.
                if self.current_modifiers.control() {
                    return self.handle_thread_list(ThreadListMessage::ToggleThread(idx));
                }
                if self.current_modifiers.shift() {
                    return self.handle_thread_list(ThreadListMessage::RangeSelectThread(idx));
                }
                // Plain click: clear multi-select, single-select.
                self.thread_list.clear_multi_select();
                self.handle_select_thread(idx)
            }
            ThreadListEvent::SearchQueryChanged(query) => {
                self.update(Message::SearchQueryChanged(query))
            }
            ThreadListEvent::SearchExecute => self.update(Message::SearchExecute),
            ThreadListEvent::SearchUndo => {
                if let Some(text) = self.search_query.undo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::SearchRedo => {
                if let Some(text) = self.search_query.redo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::ThreadDeselected => {
                self.clear_thread_selection();
                Task::none()
            }
            ThreadListEvent::WidenSearchScope => {
                // Widen search scope to all accounts
                self.sidebar.selected_scope = ViewScope::AllAccounts;
                let _ = self.nav_generation.next();
                self.update_thread_list_context_from_sidebar();
                self.update(Message::SearchExecute)
            }
            ThreadListEvent::TypeaheadQuery { .. } => Task::none(),
            ThreadListEvent::TypeaheadSelected(idx) => self.handle_typeahead_select(idx),
            ThreadListEvent::MultiSelectionChanged(_count) => {
                // Selection count changed - no action needed yet.
                Task::none()
            }
            ThreadListEvent::AutoAdvance { new_index } => {
                if let Some(idx) = new_index {
                    self.handle_select_thread(idx)
                } else {
                    self.clear_thread_selection();
                    Task::none()
                }
            }
            ThreadListEvent::BatchAction(_indices) => {
                // Batch email actions not yet wired to providers.
                Task::none()
            }
        }
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_reading_pane(&mut self, msg: ReadingPaneMessage) -> Task<Message> {
        let (task, event) = self.reading_pane.update(msg);
        let mut tasks = vec![task.map(Message::ReadingPane)];
        if let Some(evt) = event {
            tasks.push(self.handle_reading_pane_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_reading_pane_event(
        &mut self,
        event: ReadingPaneEvent,
    ) -> Task<Message> {
        match event {
            ReadingPaneEvent::AttachmentCollapseChanged {
                thread_key,
                collapsed,
            } => {
                // thread_key format is "account_id:thread_id"
                let Some((account_id, thread_id)) = thread_key.split_once(':') else {
                    return Task::none();
                };
                // Phase 6a: persist via Service IPC instead of UI-side write.
                // The reading-pane UI state is already updated through the
                // `ReadingPaneEvent` flow, so no eager-flip rollback is
                // needed here; we just persist + log on failure. The
                // policy is "log-only" because the surface is one bool
                // per thread - surfacing a status-bar error on every
                // failed collapse-toggle would be more annoying than
                // helpful, and the next reload will catch up.
                let Some(client) = self.service_client.as_ref().cloned() else {
                    log::warn!(
                        "thread_ui_state.set: no ServiceClient yet; collapse not persisted"
                    );
                    return Task::none();
                };
                let account_id = account_id.to_string();
                let thread_id = thread_id.to_string();
                Task::perform(
                    async move {
                        client
                            .set_thread_ui_state(account_id, thread_id, Some(collapsed))
                            .await
                    },
                    |result| {
                        if let Err(e) = result {
                            log::warn!("thread_ui_state.set failed: {e}");
                        }
                        Message::Noop
                    },
                )
            }
            ReadingPaneEvent::OpenMessagePopOut { message_index } => {
                self.open_message_view_window(message_index)
            }
            ReadingPaneEvent::ReplyToMessage { message_index } => self.handle_reading_pane_compose(
                message_index,
                ComposeMode::Reply {
                    original_subject: self.current_subject(),
                },
            ),
            ReadingPaneEvent::ReplyAllToMessage { message_index } => self
                .handle_reading_pane_compose(
                    message_index,
                    ComposeMode::ReplyAll {
                        original_subject: self.current_subject(),
                    },
                ),
            ReadingPaneEvent::ForwardMessage { message_index } => self.handle_reading_pane_compose(
                message_index,
                ComposeMode::Forward {
                    original_subject: self.current_subject(),
                },
            ),
            ReadingPaneEvent::EditContact { email } => {
                // Open the contact editor in settings for this email.
                // Find or create the contact, then open settings with editor.
                self.open_contact_editor_for_email(email)
            }
            ReadingPaneEvent::CreateEventFromEmail { message_index } => {
                self.create_event_from_email(message_index)
            }
            ReadingPaneEvent::ToggleStar => {
                self.update(Message::ExecuteCommand(CommandId::EmailStar))
            }
        }
    }

    /// Create a calendar event pre-filled from the given email message.
    pub(crate) fn create_event_from_email(&mut self, message_index: usize) -> Task<Message> {
        use crate::ui::calendar::{CalendarEventData, CalendarWorkflow, EditorSession};
        use chrono::Timelike;

        let msg = self.reading_pane.thread_messages.get(message_index);
        let Some(msg) = msg else { return Task::none() };

        let title = msg.subject.clone().unwrap_or_default();
        let description = msg.snippet.clone().unwrap_or_default();

        // Pre-fill attendees from To/Cc addresses.
        let today = chrono::Local::now().date_naive();
        let hour = chrono::Local::now().time().hour();
        let mut event = CalendarEventData::new_at(today, hour.min(22));
        event.title = title;
        event.description = description;

        // Set the account_id from the message's actual account.
        let account_id = Some(msg.account_id.clone());
        event.account_id = account_id.clone();

        // Pre-assign calendar when unambiguous for this account.
        if event.calendar_id.is_none()
            && let Some(ref acct) = account_id
        {
            let account_cals: Vec<_> = self
                .calendar
                .calendars
                .iter()
                .filter(|c| &c.account_id == acct)
                .collect();
            if account_cals.len() == 1 {
                event.calendar_id = Some(account_cals[0].id.clone());
            }
        }

        let session = EditorSession::new(event);
        // Workflow first, then surface.
        self.calendar.workflow = CalendarWorkflow::CreatingEvent {
            account_id,
            session,
        };
        self.calendar.sync_surfaces();

        // If calendar is popped out, focus that window instead of switching main to calendar.
        if let Some((&win_id, _)) = self
            .pop_out_windows
            .iter()
            .find(|(_, w)| matches!(w, crate::pop_out::PopOutWindow::Calendar(_)))
        {
            return iced::window::gain_focus(win_id);
        }

        // Otherwise switch to calendar mode to show the editor.
        self.app_mode = AppMode::Calendar;
        self.reload_calendar_events()
    }

    /// Returns the window ID of the calendar pop-out, if one exists.
    pub(crate) fn calendar_pop_out_id(&self) -> Option<iced::window::Id> {
        self.pop_out_windows
            .iter()
            .find(|(_, w)| matches!(w, PopOutWindow::Calendar(_)))
            .map(|(&id, _)| id)
    }

    /// Dismiss all mutually exclusive overlays (palette, settings, calendar
    /// overlays, add-account wizard). Call before opening a new overlay.
    pub(crate) fn dismiss_overlays(&mut self) {
        if self.palette.is_open() {
            self.palette.close();
        }
        if self.show_settings {
            self.close_settings();
        }
        self.calendar.workflow = crate::ui::calendar::CalendarWorkflow::Idle;
        self.calendar.sync_surfaces();
        self.add_account_wizard = None;
    }

    /// Open settings to a specific tab. Handles the full protocol:
    /// dismiss conflicting overlays, show_settings, sheet reset, animation
    /// reset, tab, begin_editing.
    pub(crate) fn open_settings(&mut self, tab: crate::ui::settings::Tab) {
        self.dismiss_overlays();
        self.show_settings = true;
        self.settings.active_sheet = None;
        self.settings
            .sheet_anim
            .go_mut(false, iced::time::Instant::now());
        self.settings.active_tab = tab;
        self.settings.begin_editing();
    }

    /// Close settings, committing preference changes.
    pub(crate) fn close_settings(&mut self) {
        self.settings.commit_preferences();
        self.show_settings = false;
    }

    /// Open the contact editor in settings for a specific email address.
    /// Navigates to Settings > People and opens the editor, creating a
    /// new local contact if none exists for that email.
    pub(crate) fn open_contact_editor_for_email(&mut self, email: String) -> Task<Message> {
        self.open_settings(crate::ui::settings::types::Tab::People);

        // Look up existing contact or create new editor state
        let found = self
            .settings
            .contacts
            .iter()
            .find(|c| c.email.eq_ignore_ascii_case(&email));

        if let Some(contact) = found {
            let id = contact.id.clone();
            self.settings.open_contact_editor(&id);
        } else {
            // Create a new editor pre-populated with the email
            self.settings.open_new_contact_editor();
            if let Some(ref mut editor) = self.settings.contact_editor {
                editor.email.set_text(email);
            }
        }

        // Load contacts for the settings view
        self.handle_load_contacts(self.settings.contact_filter.clone())
    }

    /// Get the subject of the currently selected thread.
    pub(crate) fn current_subject(&self) -> String {
        self.thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .and_then(|t| t.subject.clone())
            .unwrap_or_default()
    }

    /// Open a compose window from a reading pane Reply/ReplyAll/Forward action.
    pub(crate) fn handle_reading_pane_compose(
        &mut self,
        message_index: usize,
        mode: ComposeMode,
    ) -> Task<Message> {
        // Clone all data upfront to avoid borrow checker conflicts with &mut self.
        let msg = self
            .reading_pane
            .thread_messages
            .get(message_index)
            .cloned();
        let to_email = msg.as_ref().and_then(|m| m.from_address.clone());
        let to_name = msg.as_ref().and_then(|m| m.from_name.clone());
        let cc_emails = msg.as_ref().and_then(|m| m.cc_addresses.clone());
        let thread_id = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .map(|t| t.id.clone());
        let message_id = msg.as_ref().map(|m| m.id.clone());
        let snippet = msg.as_ref().and_then(|m| m.snippet.clone());

        let state = pop_out::compose::ComposeState::new_reply(
            &self.sidebar.accounts,
            &mode,
            to_email.as_deref(),
            to_name.as_deref(),
            cc_emails.as_deref(),
            snippet.as_deref(),
            thread_id.as_deref(),
            message_id.as_deref(),
        );

        self.open_compose_window_with_state(state, mode)
    }

    pub(crate) fn handle_status_bar(&mut self, msg: StatusBarMessage) -> Task<Message> {
        let (task, event) = self.status_bar.update(msg);
        let mut tasks = vec![task.map(Message::StatusBar)];
        if let Some(evt) = event {
            tasks.push(self.handle_status_bar_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_status_bar_event(&mut self, event: StatusBarEvent) -> Task<Message> {
        match event {
            StatusBarEvent::RequestReauth { account_id } => {
                self.handle_open_reauth_wizard(account_id)
            }
        }
    }

    pub(crate) fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::Progress {
                account_id,
                phase,
                current,
                total,
            } => {
                log::info!("Sync progress: account={account_id} phase={phase} {current}/{total}");
                let email = self.email_for_account(&account_id);
                self.status_bar
                    .report_sync_progress(account_id, email, current, total, phase);
            }
            SyncEvent::Complete { account_id } => {
                log::info!("Sync complete: account={account_id}");
                self.status_bar.report_sync_complete(&account_id);
                // Clear connection failure warnings on successful sync.
                self.status_bar.clear_warning(&account_id);
            }
            SyncEvent::Error { account_id, error } => {
                log::warn!("Sync error: account={account_id} error={error}");
                let email = self.email_for_account(&account_id);
                self.status_bar.set_warning(AccountWarning {
                    account_id,
                    email,
                    kind: WarningKind::ConnectionFailure { message: error },
                });
            }
        }
    }

    /// Look up the email address for an account ID from the sidebar's
    /// account list. Returns the account ID itself if not found.
    pub(crate) fn email_for_account(&self, account_id: &str) -> String {
        self.sidebar
            .accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.clone())
            .unwrap_or_else(|| account_id.to_string())
    }

    /// Spawn an async task that checks whether any account has an active
    /// auto-reply and delivers the result as `Message::AutoReplyChecked`.
    pub(crate) fn check_auto_reply_status(&self) -> Task<Message> {
        let db = std::sync::Arc::clone(&self.db);
        Task::perform(
            async move { db.any_auto_response_active().await },
            |result| Message::AutoReplyChecked(result.unwrap_or(false)),
        )
    }

    pub(crate) fn handle_settings(&mut self, msg: SettingsMessage) -> Task<Message> {
        let (task, event) = self.settings.update(msg);
        let mut tasks = vec![task.map(Message::Settings)];
        if let Some(evt) = event {
            tasks.push(self.handle_settings_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_settings_event(&mut self, event: SettingsEvent) -> Task<Message> {
        match event {
            SettingsEvent::Closed => {
                self.close_settings();
                Task::none()
            }
            SettingsEvent::PreferencesCommitted => {
                // Phase 6a: persist via Service IPC instead of UI-side
                // transaction. Atomicity is preserved Service-side - the
                // handler writes all values in one `unchecked_transaction`
                // so a partial commit on failure is impossible. Failure
                // policy is log-only (the user already sees the new
                // values reflected; if the persist fails, the next boot
                // shows the old values and the user can re-commit).
                let prefs = self.settings.committed_preferences.clone();
                let Some(client) = self.service_client.as_ref().cloned() else {
                    log::warn!(
                        "settings.set: no ServiceClient yet; preferences not persisted"
                    );
                    return Task::none();
                };
                let values = vec![
                    SettingValue::ShowSyncStatus(prefs.sync_status_bar),
                    SettingValue::BlockRemoteImages(prefs.block_remote_images),
                    SettingValue::PhishingDetectionEnabled(prefs.phishing_detection),
                    SettingValue::PhishingSensitivity(prefs.phishing_sensitivity.clone()),
                    SettingValue::Theme(prefs.theme.clone()),
                    SettingValue::FontSize(prefs.font_size.clone()),
                    SettingValue::ReadingPanePosition(prefs.reading_pane_position.clone()),
                ];
                Task::perform(
                    async move { client.set_settings(values).await },
                    |result| {
                        if let Err(e) = result {
                            log::warn!("settings.set failed: {e}");
                        }
                        Message::Noop
                    },
                )
            }
            SettingsEvent::PreferencesDiscarded => {
                // Live fields are already restored to committed state by Settings.
                Task::none()
            }
            SettingsEvent::DateDisplayChanged(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }
            SettingsEvent::OpenAddAccountWizard => self.handle_open_add_account_wizard(),
            SettingsEvent::DeleteAccount(account_id) => self.handle_delete_account(account_id),
            SettingsEvent::SaveAccountChanges { account_id, params } => {
                self.handle_save_account_changes(account_id, params)
            }
            SettingsEvent::SaveSignature(req) => {
                handlers::signatures::handle_save_signature(
                    self.service_client.clone(),
                    req,
                )
                .map(Message::SignatureOp)
            }
            SettingsEvent::DeleteSignature(id) => {
                handlers::signatures::handle_delete_signature(
                    self.service_client.clone(),
                    id,
                )
                .map(Message::SignatureOp)
            }
            SettingsEvent::ReorderSignatures(ordered_ids) => {
                handlers::signatures::handle_reorder_signatures(
                    self.service_client.clone(),
                    ordered_ids,
                )
                .map(Message::SignatureOp)
            }
            SettingsEvent::LoadContacts(filter) => self.handle_load_contacts(filter),
            SettingsEvent::LoadGroups(filter) => self.handle_load_groups(filter),
            SettingsEvent::SaveContact(entry) => self.handle_save_contact(entry),
            SettingsEvent::DeleteContact(id) => self.handle_delete_contact(id),
            SettingsEvent::SaveGroup(group, members) => self.handle_save_group(group, members),
            SettingsEvent::DeleteGroup(id) => self.handle_delete_group(id),
            SettingsEvent::LoadGroupMembers(group_id) => self.handle_load_group_members(group_id),
            SettingsEvent::ExecuteContactImport {
                contacts,
                account_id,
                update_existing,
            } => self.handle_import_contacts(contacts, account_id, update_existing),
            SettingsEvent::ReorderAccounts(orders) => self.handle_reorder_accounts(orders),
            SettingsEvent::ReauthenticateAccount(account_id) => {
                self.handle_open_reauth_wizard(account_id)
            }
        }
    }

    pub(crate) fn handle_add_account(&mut self, msg: AddAccountMessage) -> Task<Message> {
        let wizard = match self.add_account_wizard.as_mut() {
            Some(w) => w,
            None => return Task::none(),
        };

        let (task, event) = wizard.update(msg);
        let mut tasks = vec![task.map(Message::AddAccount)];

        if let Some(evt) = event {
            tasks.push(self.handle_add_account_event(evt));
        }
        Task::batch(tasks)
    }

    pub(crate) fn handle_signature_op(
        &mut self,
        result: handlers::SignatureResult,
    ) -> Task<Message> {
        // Phase 6a: each IPC method has its own ack variant. The
        // post-ack behavior is the same on the happy path
        // (re-list so the settings UI reflects the canonical
        // Service-committed state), but per-variant arms keep the
        // door open for per-method handling later (e.g. surfacing a
        // toast on create with the new id, or rolling back a
        // failed reorder).
        match result {
            handlers::SignatureResult::Loaded(Ok(sigs)) => {
                self.settings.signatures = sigs;
                Task::none()
            }
            handlers::SignatureResult::Loaded(Err(e)) => {
                log::error!("Failed to load signatures: {e}");
                Task::none()
            }
            handlers::SignatureResult::CreatedAck(_)
            | handlers::SignatureResult::UpdatedAck(_)
            | handlers::SignatureResult::DeletedAck(_)
            | handlers::SignatureResult::ReorderedAck(_) => {
                handlers::signatures::load_signatures_async(&self.db).map(Message::SignatureOp)
            }
        }
    }

    pub(crate) fn handle_delete_account(&mut self, account_id: String) -> Task<Message> {
        // Phase 3 task 16: cancel any in-flight Service-side sync via
        // `cancel_and_await` *inside* the deletion task below (it must
        // happen before the DB delete fires so a sync mid-persist
        // cannot write into the about-to-be-deleted account). The old
        // `sync_handles.abort()` codepath is gone with the
        // `sync_handles` field; the Service owns the cancellation
        // token now and `cancel_and_await` blocks until the runner
        // observes the token at its next checkpoint and emits
        // `SyncCompleted { Cancelled }`.

        // Close compose and message-view pop-outs belonging to the deleted account.
        let windows_to_close: Vec<iced::window::Id> = self
            .pop_out_windows
            .iter()
            .filter(|(_, w)| match w {
                PopOutWindow::Compose(state) => state
                    .from_account
                    .as_ref()
                    .is_some_and(|a| a.id == account_id),
                PopOutWindow::MessageView(state) => state.account_id == account_id,
                PopOutWindow::Calendar(_) => false,
            })
            .map(|(&id, _)| id)
            .collect();
        let mut close_tasks: Vec<Task<Message>> = Vec::new();
        for win_id in windows_to_close {
            self.pop_out_windows.remove(&win_id);
            close_tasks.push(iced::window::close(win_id));
        }

        // If the deleted account is referenced by the current scope, revert to All Accounts
        let scope_references_account = match &self.sidebar.selected_scope {
            ViewScope::Account(id) => *id == account_id,
            ViewScope::SharedMailbox {
                account_id: aid, ..
            }
            | ViewScope::PublicFolder {
                account_id: aid, ..
            } => *aid == account_id,
            ViewScope::AllAccounts => false,
        };
        if scope_references_account {
            self.sidebar.selected_scope = ViewScope::AllAccounts;
        }

        let db = Arc::clone(&self.db);
        let body_store = self.body_store.clone();
        let inline_image_store = self.inline_image_store.clone();
        let search = self.search_state.clone();
        let app_data_dir = crate::APP_DATA_DIR.get().expect("APP_DATA_DIR not set").clone();
        let service_client = self.service_client.clone();

        let delete_task = Task::perform(
            async move {
                // Phase 3 task 16: cancel any in-flight Service-side
                // sync first. Tolerates ServiceCrashed - a dead Service
                // provably has no in-flight writers, so the delete
                // proceeds. Other IPC errors are logged and the delete
                // proceeds anyway (best-effort; the Service-side
                // invariant pass on next boot reconciles).
                if let Some(client) = service_client.as_ref() {
                    match client.cancel_and_await(&account_id).await {
                        Ok(_) => {}
                        Err(crate::service_client::ClientError::ServiceCrashed) => {}
                        Err(error) => {
                            log::warn!(
                                "cancel_and_await({account_id}) before delete: {error}; proceeding",
                            );
                        }
                    }
                }

                // Phase 1: gather cleanup data + ref-checks + delete account row
                // (synchronous, inside one write-connection call so CASCADE hasn't
                // fired yet when we query attachment rows)
                let plan = db
                    .with_write_conn(move |conn| {
                        rtsk::account::delete::delete_account_orchestrate(conn, &account_id)
                    })
                    .await?;

                // Phase 2: best-effort cleanup of external stores
                let mut report = rtsk::account::types::AccountDeletionCleanupReport::default();

                // Body store
                if let Some(ref bs) = body_store {
                    match bs.delete(plan.data.message_ids.clone()).await {
                        Ok(n) => report.bodies_deleted = n,
                        Err(e) => log::error!("Account deletion: body store cleanup failed: {e}"),
                    }
                } else {
                    log::warn!("Account deletion: body store unavailable, skipping cleanup");
                }

                // Inline image store - only delete hashes not shared with other accounts
                if let Some(ref iis) = inline_image_store {
                    let to_delete: Vec<String> = plan
                        .data
                        .inline_hashes
                        .into_iter()
                        .filter(|h| !plan.shared_inline_hashes.contains(h))
                        .collect();
                    if !to_delete.is_empty() {
                        match iis.delete_hashes(to_delete).await {
                            Ok(n) => report.inline_images_deleted = n,
                            Err(e) => {
                                log::error!("Account deletion: inline image cleanup failed: {e}");
                            }
                        }
                    }
                } else {
                    log::warn!(
                        "Account deletion: inline image store unavailable, skipping cleanup"
                    );
                }

                // Attachment file cache - only delete files not shared with other accounts
                for (path, hash) in &plan.data.cached_files {
                    if plan.shared_cache_hashes.contains(hash) {
                        continue;
                    }
                    match rtsk::attachment_cache::remove_cached_relative(&app_data_dir, path) {
                        Ok(()) => report.cache_files_deleted += 1,
                        Err(e) => report.cache_file_errors.push((path.clone(), e)),
                    }
                }

                // Search index cleanup deferred. Phase 3 task 4 strips
                // writer ownership from `SearchReadState`; the UI no
                // longer has a `SearchWriteHandle` here. Phase 3 task 16
                // routes account-deletion through `cancel_and_await`,
                // which lets the Service do this cleanup before the
                // delete returns. Until then, the boot-time invariant
                // pass (Phase 3 task 11) drops orphans by account on
                // dirty boot, and the next sync's reindex makes the
                // search results consistent again.
                let _ = (&search, &plan.data.message_ids);
                report.search_cleaned = false;

                log::info!(
                    "Account deleted: {} bodies, {} inline images, {} cache files cleaned",
                    report.bodies_deleted,
                    report.inline_images_deleted,
                    report.cache_files_deleted,
                );
                if !report.cache_file_errors.is_empty() {
                    log::warn!(
                        "Account deletion: {} cache files failed to delete",
                        report.cache_file_errors.len()
                    );
                }

                Ok(())
            },
            Message::AccountDeleted,
        );

        close_tasks.push(delete_task);
        Task::batch(close_tasks)
    }

    pub(crate) fn handle_save_account_changes(
        &mut self,
        account_id: String,
        params: rtsk::db::queries_extra::UpdateAccountParams,
    ) -> Task<Message> {
        // Phase 6a: account.update IPC. Convert the existing internal
        // UpdateAccountParams (carries no id; takes the id from the
        // function arg) into the wire shape (id-bearing).
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("account.update: no ServiceClient yet; ignoring update");
            return Task::none();
        };
        let wire = service_api::AccountUpdateParams {
            id: account_id,
            account_name: params.account_name,
            display_name: params.display_name,
            account_color: params.account_color,
            caldav_url: params.caldav_url,
            caldav_username: params.caldav_username,
            caldav_password: params.caldav_password,
        };
        Task::perform(
            async move {
                client
                    .update_account(wire)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::AccountUpdated,
        )
    }

    pub(crate) fn handle_reorder_accounts(
        &mut self,
        orders: Vec<(String, i64)>,
    ) -> Task<Message> {
        // Phase 6a: account.reorder IPC. Same staleness tolerance as
        // signature.reorder - rapid drag-reorder clicks may land out
        // of order; next reload reconciles. Per-entity ordering token
        // is the documented escape hatch if a real bug shows up.
        let Some(client) = self.service_client.as_ref().cloned() else {
            log::warn!("account.reorder: no ServiceClient yet; ignoring reorder");
            return Task::none();
        };
        Task::perform(
            async move {
                client
                    .reorder_accounts(orders)
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::AccountUpdated,
        )
    }

    pub(crate) fn handle_clear_all_pinned_searches(&mut self) -> Task<Message> {
        self.pinned_searches.clear();
        self.sidebar.active_pinned_search = None;
        self.sidebar.pinned_searches.clear();
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.delete_all_pinned_searches().await.map(|_| ()) },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to clear pinned searches: {e}");
                }
                Message::Noop
            },
        )
    }

    pub(crate) fn handle_window_close(&mut self, id: iced::window::Id) -> Task<Message> {
        if id == self.main_window_id {
            log::info!("Main window closing, saving state");
            let data_dir = crate::APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
            self.window.sidebar_width = self.sidebar_width;
            self.window.thread_list_width = self.thread_list_width;
            self.window.right_sidebar_open = self.right_sidebar_open;
            self.window.save(data_dir);
            self.save_session_state();
            // Save dirty compose drafts synchronously before destroying windows
            let compose_ids: Vec<_> = self
                .pop_out_windows
                .iter()
                .filter_map(|(&win_id, w)| {
                    matches!(w, PopOutWindow::Compose(s) if s.draft_dirty).then_some(win_id)
                })
                .collect();
            for win_id in compose_ids {
                self.save_compose_draft_sync(win_id);
            }
            let mut tasks: Vec<Task<Message>> = self
                .pop_out_windows
                .keys()
                .map(|&win_id| iced::window::close(win_id))
                .collect();
            self.pop_out_windows.clear();
            tasks.push(iced::window::close(id));
            if let Some(service_client) = self.service_client.clone() {
                tasks.push(Task::perform(
                    async move {
                        service_client
                            .shutdown()
                            .await
                            .map_err(|error| error.to_string())
                    },
                    Message::ServiceShutdownComplete,
                ));
            } else {
                tasks.push(iced::exit());
            }
            return Task::batch(tasks);
        }

        if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&id) {
            if state.has_user_content() && !state.discard_confirm_open {
                // Show discard confirmation instead of closing silently
                state.discard_confirm_open = true;
                return Task::none();
            }
            // Either no content or user already confirmed - save and close
            if !self.save_compose_draft_sync(id) {
                log::warn!("Compose draft save failed, aborting window close");
                return Task::none();
            }
        }
        // Calendar pop-out closing - calendar becomes available in main window again.
        // (No state change needed - mode toggle just works.)
        self.pop_out_windows.remove(&id);
        iced::window::close(id)
    }

    #[cfg_attr(feature = "hotpath", hotpath::measure)]
    pub(crate) fn handle_select_thread(&mut self, idx: usize) -> Task<Message> {
        let thread = self.thread_list.threads.get(idx);
        if let Some(t) = thread {
            log::debug!("Thread selected: {}", t.id);
        }

        // Local drafts open in a compose pop-out instead of the reading pane.
        if let Some(t) = thread
            && t.is_local_draft
        {
            let draft_id = t.id.clone();
            let db = Arc::clone(&self.db);
            return Task::perform(
                async move {
                    let core_db = db.read_db_state();
                    rtsk::db::queries_extra::db_get_local_draft(&core_db, draft_id).await
                },
                Message::LocalDraftLoaded,
            );
        }

        self.reading_pane.set_thread(thread);

        // Public folder items aren't real threads - skip detail loading.
        if matches!(self.sidebar.selected_scope, ViewScope::PublicFolder { .. }) {
            return Task::none();
        }

        // Set search highlight terms when in search mode
        if self.thread_list.mode == ui::thread_list::ThreadListMode::Search {
            let query = self.search_query.text().to_string();
            let parsed = smart_folder::parse_query(&query);
            self.reading_pane.search_highlight_terms = parsed
                .free_text
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            self.reading_pane.search_highlight_terms.clear();
        }

        let thread_gen = self.thread_generation.next();
        if let Some(thread) = thread {
            let account_id = thread.account_id.clone();
            let thread_id = thread.id.clone();
            let load_gen = thread_gen;

            // Use core's thread detail if body store is available,
            // otherwise fall back to the old separate queries.
            if let Some(ref body_store) = self.body_store {
                let db = Arc::clone(&self.db);
                let bs = body_store.clone();
                let iis = self.inline_image_store.clone();
                return Task::perform(
                    async move {
                        let r = db::threads::load_thread_detail(
                            &db,
                            &bs,
                            iis.as_ref(),
                            account_id,
                            thread_id,
                        )
                        .await;
                        (load_gen, r)
                    },
                    |(g, result)| Message::ThreadDetailLoaded(g, result),
                );
            }
        }
        Task::none()
    }

    pub(crate) fn handle_accounts_loaded(
        &mut self,
        accounts: Vec<db::Account>,
    ) -> Task<Message> {
        self.sidebar.accounts = accounts;
        if self.sidebar.accounts.is_empty() {
            self.no_accounts = true;
            self.add_account_wizard = Some(
                crate::ui::add_account::AddAccountWizard::new_first_launch(
                    Arc::clone(&self.db),
                    self.service_client.clone(),
                ),
            );
            self.status = "Welcome".to_string();
            return Task::none();
        }
        self.no_accounts = false;
        self.settings.managed_accounts = self
            .sidebar
            .accounts
            .iter()
            .map(|a| ui::settings::ManagedAccount {
                id: a.id.clone(),
                email: a.email.clone(),
                provider: a.provider.clone(),
                account_name: a.account_name.clone(),
                account_color: a.account_color.clone(),
                display_name: a.display_name.clone(),
                last_sync_at: a.last_sync_at,
                health: ui::settings::compute_health(
                    a.last_sync_at,
                    a.token_expires_at,
                    a.is_active,
                ),
            })
            .collect();
        if let Some(first) = self.sidebar.accounts.first() {
            self.sidebar.selected_scope = ViewScope::Account(first.id.clone());
        }
        self.status = format!("Loaded {} accounts", self.sidebar.accounts.len());
        let sig_task =
            handlers::signatures::load_signatures_async(&self.db).map(Message::SignatureOp);
        let sync_task = self.sync_all_accounts();
        let auto_reply_task = self.check_auto_reply_status();
        // Per-account thread_participants backfill used to run from here.
        // As of Phase 1.5 it runs Service-side during the boot sequence
        // (BootPhase::BackfillingThreadParticipants); the helper is
        // idempotent so future-account-creation backfill becomes a Phase 2
        // action-pipeline concern.
        //
        // JMAP push setup also used to run here; as of Phase 4 it runs
        // Service-side from a post-`boot.ready` runtime task in
        // `dispatch.rs::spawn_post_ready_push_startup`. The
        // `sync.start_account` IPC piggybacks push setup for newly
        // added accounts (Phase 4 task 6).
        Task::batch([
            self.load_navigation_and_threads(),
            sig_task,
            sync_task,
            auto_reply_task,
        ])
    }

    pub(crate) fn update_thread_list_context_from_sidebar(&mut self) {
        let folder_name = self
            .sidebar
            .selection
            .navigation_folder_id()
            .and_then(|nav_id| {
                self.sidebar.nav_state.as_ref().and_then(|ns| {
                    ns.folders
                        .iter()
                        .find(|f| f.id == nav_id)
                        .map(|f| f.name.clone())
                })
            })
            .unwrap_or_else(|| "Inbox".to_string());
        let scope_name = match &self.sidebar.selected_scope {
            ViewScope::AllAccounts => "All".to_string(),
            ViewScope::Account(id) => self
                .sidebar
                .accounts
                .iter()
                .find(|a| a.id == *id)
                .and_then(|a| a.display_name.as_deref().or(Some(a.email.as_str())))
                .unwrap_or("Account")
                .to_string(),
            ViewScope::SharedMailbox { mailbox_id, .. } => self
                .sidebar
                .shared_mailboxes
                .iter()
                .find(|sm| sm.mailbox_id == *mailbox_id)
                .and_then(|sm| sm.display_name.as_deref())
                .unwrap_or(mailbox_id.as_str())
                .to_string(),
            ViewScope::PublicFolder { folder_id, .. } => self
                .sidebar
                .pinned_public_folders
                .iter()
                .find(|pf| pf.folder_id == *folder_id)
                .map(|pf| pf.display_name.as_str())
                .unwrap_or(folder_id.as_str())
                .to_string(),
        };
        self.thread_list.set_context(folder_name, scope_name);
    }
}
