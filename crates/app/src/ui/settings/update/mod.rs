use iced::time::Instant;
use iced::Task;

use crate::component::Component;
use crate::db::DateDisplay;
use crate::ui::undoable::UndoableText;
use rte::EditorState as RteEditorState;

use crate::ui::settings::tabs::settings_view;
use crate::ui::settings::types::*;

mod accounts;
mod contacts_groups;
mod helpers;
mod list_drag;
mod signatures;
mod undo_redo;

impl Component for Settings {
    type Message = SettingsMessage;
    type Event = SettingsEvent;

    fn update(
        &mut self,
        message: SettingsMessage,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        match message {
            SettingsMessage::Close => {
                self.commit_preferences();
                return (Task::none(), Some(SettingsEvent::Closed));
            }
            SettingsMessage::SavePreferences => {
                self.commit_preferences();
                return (Task::none(), Some(SettingsEvent::PreferencesCommitted));
            }
            SettingsMessage::CancelPreferences => {
                self.discard_preferences();
                return (Task::none(), Some(SettingsEvent::PreferencesDiscarded));
            }
            SettingsMessage::FocusInput(id) => {
                return (iced::widget::operation::focus(id), None);
            }
            SettingsMessage::CopyToClipboard(contents) => {
                return (iced::clipboard::write(contents), None);
            }
            SettingsMessage::DateDisplayChanged(v) => {
                self.date_display = match v.as_str() {
                    "Absolute" => DateDisplay::Absolute,
                    _ => DateDisplay::RelativeOffset,
                };
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.date_display = self.date_display;
                }
                self.open_select = None;
                return (
                    Task::none(),
                    Some(SettingsEvent::DateDisplayChanged(self.date_display)),
                );
            }
            SettingsMessage::AddAccountFromSettings => {
                return (Task::none(), Some(SettingsEvent::OpenAddAccountWizard));
            }
            SettingsMessage::AccountCardClicked(id) => {
                self.open_account_editor(&id);
                return (Task::none(), None);
            }
            SettingsMessage::CloseAccountEditor => {
                self.editing_account = None;
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
                return (Task::none(), None);
            }
            SettingsMessage::SaveAccountEditor => {
                return self.handle_account_editor_save();
            }
            SettingsMessage::DeleteAccountRequested(id) => {
                if let Some(ref mut editor) = self.editing_account
                    && editor.account_id == id
                {
                    editor.show_delete_confirmation = true;
                }
                return (Task::none(), None);
            }
            SettingsMessage::DeleteAccountConfirmed(id) => {
                self.editing_account = None;
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
                return (Task::none(), Some(SettingsEvent::DeleteAccount(id)));
            }
            SettingsMessage::DeleteAccountCancelled => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.show_delete_confirmation = false;
                }
                return (Task::none(), None);
            }
            SettingsMessage::ReauthenticateAccount(id) => {
                return (Task::none(), Some(SettingsEvent::ReauthenticateAccount(id)));
            }
            SettingsMessage::SignatureEditorSave => {
                return self.handle_signature_save();
            }
            SettingsMessage::SignatureDelete(ref id) => {
                let need_open = self
                    .signature_editor
                    .as_ref()
                    .is_none_or(|e| e.signature_id.as_deref() != Some(id.as_str()));
                if need_open
                    && let Some(sig) = self.signatures.iter().find(|s| s.id == *id)
                {
                    self.signature_editor = Some(SignatureEditorState {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                        name: UndoableText::with_initial(&sig.name),
                        body_editor: RteEditorState::from_html(&sig.body_html),
                        is_default: sig.is_default,
                        is_reply_default: sig.is_reply_default,
                        dirty: false,
                    });
                    self.active_sheet = Some(SettingsSheetPage::EditSignature {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                    });
                    self.sheet_anim.go_mut(true, Instant::now());
                }
                self.confirm_delete_signature = Some(id.clone());
                return (Task::none(), None);
            }
            SettingsMessage::SignatureDeleteConfirmed(id) => {
                self.confirm_delete_signature = None;
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
                self.signature_editor = None;
                return (Task::none(), Some(SettingsEvent::DeleteSignature(id)));
            }
            SettingsMessage::SignatureDeleteCancelled => {
                self.confirm_delete_signature = None;
                return (Task::none(), None);
            }
            SettingsMessage::ListDragMove(list_id, point) => {
                return (self.handle_drag_move(&list_id, point), None);
            }
            SettingsMessage::AccountDragMove(point) => {
                return self.handle_account_drag_move(point);
            }
            SettingsMessage::AccountDragEnd => {
                return self.handle_account_drag_end();
            }
            SettingsMessage::SelectTab(Tab::People) => {
                self.active_tab = Tab::People;
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
                return (
                    Task::none(),
                    Some(SettingsEvent::LoadContacts(self.contact_filter.clone())),
                );
            }
            SettingsMessage::ContactEditorSave => {
                return self.handle_contact_save();
            }
            SettingsMessage::ContactEditorFieldChanged(_, _) => {
                self.handle_remaining_message(message);
                if let Some(ref editor) = self.contact_editor {
                    let is_local = editor.source.as_deref().is_none_or(|s| s == "user");
                    if is_local && editor.contact_id.is_some() {
                        return self.handle_contact_save();
                    }
                }
                return (Task::none(), None);
            }
            SettingsMessage::ContactDelete(id) => {
                self.confirm_delete_contact = Some(id);
                return (Task::none(), None);
            }
            SettingsMessage::ContactConfirmDelete(id) => {
                self.confirm_delete_contact = None;
                return self.handle_contact_delete(id);
            }
            SettingsMessage::ContactCancelDelete => {
                self.confirm_delete_contact = None;
                return (Task::none(), None);
            }
            SettingsMessage::GroupEditorSave => {
                return self.handle_group_save();
            }
            SettingsMessage::GroupDelete(id) => {
                self.confirm_delete_group = Some(id);
                return (Task::none(), None);
            }
            SettingsMessage::GroupConfirmDelete(id) => {
                self.confirm_delete_group = None;
                return self.handle_group_delete(id);
            }
            SettingsMessage::GroupCancelDelete => {
                self.confirm_delete_group = None;
                return (Task::none(), None);
            }
            SettingsMessage::ImportContactsOpen => {
                self.import_wizard = Some(ImportWizardState::new());
                self.active_sheet = Some(SettingsSheetPage::ImportContacts);
                self.sheet_anim.go_mut(true, Instant::now());
                return (Task::none(), None);
            }
            SettingsMessage::ImportFileSelected(path, data) => {
                return (self.handle_import_file_selected(path, data), None);
            }
            SettingsMessage::ImportMappingChanged(index, field) => {
                if let Some(ref mut wizard) = self.import_wizard
                    && let Some(mapping) = wizard.mappings.get_mut(index)
                {
                    *mapping = field;
                }
                return (Task::none(), None);
            }
            SettingsMessage::ImportToggleHeader(has_header) => {
                return (self.handle_import_toggle_header(has_header), None);
            }
            SettingsMessage::ImportToggleUpdateExisting(update) => {
                if let Some(ref mut wizard) = self.import_wizard {
                    wizard.update_existing = update;
                }
                return (Task::none(), None);
            }
            SettingsMessage::ImportAccountChanged(account_id) => {
                if let Some(ref mut wizard) = self.import_wizard {
                    wizard.account_id = account_id;
                }
                return (Task::none(), None);
            }
            SettingsMessage::ImportExecute => {
                return self.handle_import_execute();
            }
            SettingsMessage::ImportExecuted(result) => {
                if let Some(ref mut wizard) = self.import_wizard {
                    match result {
                        Ok(import_result) => {
                            wizard.result = Some(import_result);
                            wizard.step = ImportStep::Summary;
                        }
                        Err(e) => {
                            log::error!("Import failed: {e}");
                            wizard.step = ImportStep::Summary;
                            wizard.result = Some(ImportResult {
                                imported: 0,
                                skipped_no_email: 0,
                                skipped_duplicate: 0,
                                updated: 0,
                                groups_created: 0,
                            });
                        }
                    }
                }
                return (Task::none(), None);
            }
            SettingsMessage::ImportBack => {
                if let Some(ref mut wizard) = self.import_wizard {
                    match wizard.step {
                        ImportStep::Mapping | ImportStep::VcfPreview => {
                            wizard.step = ImportStep::FileSelect;
                            wizard.source = None;
                            wizard.preview = None;
                            wizard.mappings.clear();
                            wizard.vcf_contacts.clear();
                        }
                        ImportStep::Summary => {
                            self.import_wizard = None;
                            self.active_sheet = None;
                            self.sheet_anim.go_mut(false, Instant::now());
                            return (
                                Task::none(),
                                Some(SettingsEvent::LoadContacts(self.contact_filter.clone())),
                            );
                        }
                        _ => {}
                    }
                }
                return (Task::none(), None);
            }
            SettingsMessage::ContactFilterChanged(v) => {
                self.contact_filter = v.clone();
                self.focused_filter = Some(FilterId::Contacts);
                return (Task::none(), Some(SettingsEvent::LoadContacts(v)));
            }
            SettingsMessage::GroupFilterChanged(v) => {
                self.group_filter = v.clone();
                self.focused_filter = Some(FilterId::Groups);
                return (Task::none(), Some(SettingsEvent::LoadGroups(v)));
            }
            SettingsMessage::FilterFocused(id) => {
                self.focused_filter = Some(id);
            }
            SettingsMessage::FilterFocusUpdated(maybe_id) => {
                self.focused_filter = maybe_id;
            }
            SettingsMessage::FilterCleared(id) => {
                match id {
                    FilterId::Contacts => {
                        self.contact_filter.clear();
                        if self.focused_filter == Some(FilterId::Contacts) {
                            self.focused_filter = None;
                        }
                        return (
                            Task::none(),
                            Some(SettingsEvent::LoadContacts(String::new())),
                        );
                    }
                    FilterId::Groups => {
                        self.group_filter.clear();
                        if self.focused_filter == Some(FilterId::Groups) {
                            self.focused_filter = None;
                        }
                        return (
                            Task::none(),
                            Some(SettingsEvent::LoadGroups(String::new())),
                        );
                    }
                    FilterId::GroupAddMembers => {
                        if let Some(ref mut editor) = self.group_editor {
                            editor.filter.clear();
                        }
                        if self.focused_filter == Some(FilterId::GroupAddMembers) {
                            self.focused_filter = None;
                        }
                    }
                    FilterId::GroupMembers => {
                        if let Some(ref mut editor) = self.group_editor {
                            editor.members_filter.clear();
                        }
                        if self.focused_filter == Some(FilterId::GroupMembers) {
                            self.focused_filter = None;
                        }
                    }
                }
            }
            SettingsMessage::GroupClick(id) => {
                self.open_group_editor(&id);
                return (Task::none(), Some(SettingsEvent::LoadGroupMembers(id)));
            }
            _ => self.handle_simple_message(message),
        }
        (Task::none(), None)
    }

    fn view(&self) -> iced::Element<'_, SettingsMessage> {
        settings_view(self)
    }
}

impl Settings {
    fn handle_simple_message(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::Noop
            | SettingsMessage::CheckForUpdates
            | SettingsMessage::OpenGithub
            | SettingsMessage::SheetAnimTick(_) => {}
            SettingsMessage::UndoInput(field) => {
                self.undo_field(field);
            }
            SettingsMessage::RedoInput(field) => {
                self.redo_field(field);
            }
            SettingsMessage::HelpHover(id) => self.hovered_help = Some(id),
            SettingsMessage::HelpUnhover(id) => {
                if self.hovered_help.as_ref() == Some(&id) {
                    self.hovered_help = None;
                }
            }
            SettingsMessage::SelectTab(tab) => {
                self.active_tab = tab;
                self.hovered_help = None;
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
            }
            SettingsMessage::ToggleSelect(field) => {
                self.open_select = if self.open_select == Some(field) {
                    None
                } else {
                    Some(field)
                };
            }
            SettingsMessage::ScaleDragged(v) => self.scale_preview = Some(v),
            SettingsMessage::ScaleReleased => {
                if let Some(v) = self.scale_preview.take() {
                    self.scale = v;
                    if let Some(ref mut prefs) = self.editing_preferences {
                        prefs.scale = v;
                    }
                }
            }
            SettingsMessage::EmailBodyBgChanged(v) => {
                let bg = EmailBodyBackground::from_label(&v);
                self.email_body_background = bg;
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.email_body_background = bg;
                }
                crate::ui::theme::set_email_body_background(bg);
                self.open_select = None;
            }
            SettingsMessage::ThemeChanged(v) => {
                self.theme = v.clone();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.theme = v;
                }
                self.open_select = None;
            }
            SettingsMessage::DensityChanged(v) => {
                self.density = v.clone();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.density = v;
                }
                self.open_select = None;
            }
            SettingsMessage::FontSizeChanged(v) => {
                self.font_size = v.clone();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.font_size = v;
                }
                self.open_select = None;
            }
            SettingsMessage::ReadingPaneChanged(v) => {
                self.reading_pane_position = v.clone();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.reading_pane_position = v;
                }
                self.open_select = None;
            }
            SettingsMessage::ThemeSelected(i) => {
                self.selected_theme = Some(i);
                self.theme = "Theme".into();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.theme = "Theme".into();
                }
            }
            SettingsMessage::ToggleSyncStatusBar(v) => {
                self.sync_status_bar = v;
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.sync_status_bar = v;
                }
            }
            SettingsMessage::ToggleBlockRemoteImages(v) => {
                self.block_remote_images = v;
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.block_remote_images = v;
                }
            }
            SettingsMessage::TogglePhishingDetection(v) => {
                self.phishing_detection = v;
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.phishing_detection = v;
                }
            }
            SettingsMessage::PhishingSensitivityChanged(v) => {
                self.phishing_sensitivity = v.clone();
                if let Some(ref mut prefs) = self.editing_preferences {
                    prefs.phishing_sensitivity = v;
                }
            }
            SettingsMessage::ToggleSendAndArchive(v) => self.send_and_archive = v,
            SettingsMessage::UndoDelayChanged(v) => {
                self.undo_delay = v;
                self.open_select = None;
            }
            SettingsMessage::DefaultReplyChanged(v) => {
                self.default_reply_mode = v;
                self.open_select = None;
            }
            SettingsMessage::MarkAsReadChanged(v) => {
                self.mark_as_read = v;
                self.open_select = None;
            }
            _ => self.handle_remaining_message(message),
        }
    }

    fn handle_remaining_message(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::ToggleNotifications(v) => self.notifications_enabled = v,
            SettingsMessage::ToggleSmartNotifications(v) => self.smart_notifications = v,
            SettingsMessage::ToggleNotifyCategory(cat) => {
                if let Some(pos) = self.notify_categories.iter().position(|c| c == &cat) {
                    self.notify_categories.remove(pos);
                } else {
                    self.notify_categories.push(cat);
                }
            }
            SettingsMessage::VipEmailChanged(v) => self.vip_email_input.set_text(v),
            SettingsMessage::AddVipSender => {
                let email = self.vip_email_input.text().trim().to_string();
                if !email.is_empty() && !self.vip_senders.contains(&email) {
                    self.vip_senders.push(email);
                    self.vip_email_input.set_text(String::new());
                }
            }
            SettingsMessage::RemoveVipSender(email) => self.vip_senders.retain(|e| e != &email),
            SettingsMessage::AiProviderChanged(v) => {
                self.ai_provider = v;
                self.open_select = None;
            }
            SettingsMessage::AiModelChanged(v) => {
                self.ai_model = v;
                self.open_select = None;
            }
            SettingsMessage::ToggleAiEnabled(v) => self.ai_enabled = v,
            SettingsMessage::ToggleAiAutoCategorize(v) => self.ai_auto_categorize = v,
            SettingsMessage::ToggleAiAutoSummarize(v) => self.ai_auto_summarize = v,
            SettingsMessage::ToggleAiAutoDraft(v) => self.ai_auto_draft = v,
            SettingsMessage::ToggleAiWritingStyle(v) => self.ai_writing_style = v,
            SettingsMessage::ToggleAiAutoArchiveUpdates(v) => self.ai_auto_archive_updates = v,
            SettingsMessage::ToggleAiAutoArchivePromotions(v) => {
                self.ai_auto_archive_promotions = v;
            }
            SettingsMessage::ToggleAiAutoArchiveSocial(v) => self.ai_auto_archive_social = v,
            SettingsMessage::ToggleAiAutoArchiveNewsletters(v) => {
                self.ai_auto_archive_newsletters = v;
            }
            SettingsMessage::AiApiKeyChanged(v) => self.ai_api_key.set_text(v),
            SettingsMessage::OllamaUrlChanged(v) => self.ai_ollama_url.set_text(v),
            SettingsMessage::OllamaModelChanged(v) => self.ai_ollama_model.set_text(v),
            SettingsMessage::ListGripPress(list_id, index) => {
                self.drag_state = Some(DragState {
                    list_id,
                    dragging_index: index,
                    start_y: -1.0,
                    is_dragging: false,
                });
            }
            SettingsMessage::AccountGripPress(index) => {
                self.account_drag = Some(AccountDragState {
                    dragging_index: index,
                    start_y: -1.0,
                    is_dragging: false,
                });
            }
            SettingsMessage::ListDragEnd(_) => self.drag_state = None,
            SettingsMessage::ListRowClick(list_id, index) if self.drag_state.is_none() => {
                let items = self.list_items_mut(&list_id);
                if let Some(item) = items.get_mut(index)
                    && let Some(ref mut enabled) = item.enabled
                {
                    *enabled = !*enabled;
                }
            }
            SettingsMessage::ListRemove(list_id, index) => {
                let items = self.list_items_mut(&list_id);
                if index < items.len() {
                    items.remove(index);
                }
            }
            SettingsMessage::ListAdd(list_id) => {
                let items = self.list_items_mut(&list_id);
                items.push(EditableItem {
                    label: format!("New item {}", items.len() + 1),
                    enabled: None,
                });
            }
            SettingsMessage::ListToggle(list_id, index, value) => {
                if let Some(item) = self.list_items_mut(&list_id).get_mut(index) {
                    item.enabled = Some(value);
                }
            }
            SettingsMessage::ListMenu(_, _) => {}
            SettingsMessage::AccountNameEditorChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.account_name.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::DisplayNameEditorChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.display_name.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::AccountColorEditorChanged(idx) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.account_color_index = Some(idx);
                    editor.dirty = true;
                }
            }
            SettingsMessage::CaldavUrlChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_url.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::CaldavUsernameChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_username.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::CaldavPasswordChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_password.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::SignatureEdit(sig_id) => {
                if let Some(sig) = self.signatures.iter().find(|s| s.id == sig_id) {
                    self.signature_editor = Some(SignatureEditorState {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                        name: UndoableText::with_initial(&sig.name),
                        body_editor: RteEditorState::from_html(&sig.body_html),
                        is_default: sig.is_default,
                        is_reply_default: sig.is_reply_default,
                        dirty: false,
                    });
                    self.active_sheet = Some(SettingsSheetPage::EditSignature {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                    });
                    self.sheet_anim.go_mut(true, Instant::now());
                }
            }
            SettingsMessage::SignatureCreate(account_id) => {
                self.signature_editor = Some(SignatureEditorState {
                    signature_id: None,
                    account_id: account_id.clone(),
                    name: UndoableText::new(),
                    body_editor: RteEditorState::new(),
                    is_default: false,
                    is_reply_default: false,
                    dirty: false,
                });
                self.active_sheet = Some(SettingsSheetPage::EditSignature {
                    signature_id: None,
                    account_id,
                });
                self.sheet_anim.go_mut(true, Instant::now());
            }
            SettingsMessage::SignatureEditorNameChanged(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.name.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::SignatureEditorBodyChanged(_) => {}
            SettingsMessage::SignatureEditorAction(action) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.body_editor.perform(action);
                    editor.dirty = true;
                }
            }
            SettingsMessage::SignatureEditorToggleDefault(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.is_default = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::SignatureEditorToggleReplyDefault(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.is_reply_default = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::OpenSheet(sheet) => {
                self.active_sheet = Some(sheet);
                self.sheet_anim.go_mut(true, Instant::now());
            }
            SettingsMessage::CloseSheet => {
                self.active_sheet = None;
                self.sheet_anim.go_mut(false, Instant::now());
                self.signature_editor = None;
                self.editing_account = None;
                self.contact_editor = None;
                self.group_editor = None;
                self.import_wizard = None;
            }
            SettingsMessage::ContactsLoaded(Ok(contacts)) => {
                self.contacts = contacts;
            }
            SettingsMessage::ContactsLoaded(Err(e)) => {
                log::error!("Failed to load contacts: {e}");
            }
            SettingsMessage::GroupsLoaded(Ok(groups)) => {
                self.groups = groups;
            }
            SettingsMessage::GroupsLoaded(Err(e)) => {
                log::error!("Failed to load groups: {e}");
            }
            SettingsMessage::GroupMembersLoaded(group_id, Ok(members)) => {
                if let Some(ref mut editor) = self.group_editor
                    && editor.group_id.as_deref() == Some(group_id.as_str())
                {
                    editor.members = members;
                }
            }
            SettingsMessage::GroupMembersLoaded(_, Err(e)) => {
                log::error!("Failed to load group members: {e}");
            }
            SettingsMessage::ContactClick(id) => {
                self.open_contact_editor(&id);
            }
            SettingsMessage::ContactCreate => {
                self.open_new_contact_editor();
            }
            SettingsMessage::ContactEditorFieldChanged(field, value) => {
                if let Some(ref mut editor) = self.contact_editor {
                    match field {
                        ContactField::DisplayName => editor.display_name.set_text(value),
                        ContactField::Email => editor.email.set_text(value),
                        ContactField::Email2 => editor.email2.set_text(value),
                        ContactField::Phone => editor.phone.set_text(value),
                        ContactField::Company => editor.company.set_text(value),
                        ContactField::Notes => editor.notes.set_text(value),
                    }
                    editor.dirty = true;
                }
            }
            SettingsMessage::ContactEditorAccountChanged(account_id) => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.account_id = account_id;
                    editor.dirty = true;
                }
            }
            SettingsMessage::ContactSaved(Ok(())) | SettingsMessage::ContactDeleted(Ok(())) => {}
            SettingsMessage::ContactSaved(Err(_)) | SettingsMessage::ContactDeleted(Err(_)) => {}
            SettingsMessage::GroupCreate => {
                self.open_new_group_editor();
            }
            SettingsMessage::GroupEditorNameChanged(v) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.name.set_text(v);
                    editor.dirty = true;
                }
            }
            SettingsMessage::GroupEditorRemoveMember(email) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.members.retain(|m| m != &email);
                    editor.dirty = true;
                }
            }
            SettingsMessage::GroupEditorAddMember(email) => {
                if let Some(ref mut editor) = self.group_editor
                    && !editor.members.contains(&email)
                {
                    editor.members.push(email);
                    editor.dirty = true;
                }
            }
            SettingsMessage::GroupEditorFilterChanged(v) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.filter = v;
                }
                self.focused_filter = Some(FilterId::GroupAddMembers);
            }
            SettingsMessage::GroupEditorMembersFilterChanged(v) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.members_filter = v;
                }
                self.focused_filter = Some(FilterId::GroupMembers);
            }
            SettingsMessage::GroupSaved(Ok(())) | SettingsMessage::GroupDeleted(Ok(())) => {}
            SettingsMessage::GroupSaved(Err(_)) | SettingsMessage::GroupDeleted(Err(_)) => {}
            _ => {}
        }
    }
}
