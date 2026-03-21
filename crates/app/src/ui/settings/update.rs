use iced::time::Instant;
use iced::{Point, Task};

use crate::component::Component;
use crate::db::DateDisplay;
use crate::ui::layout::*;
use crate::ui::undoable::UndoableText;

use super::tabs::settings_view;
use super::types::*;

// ── Component impl ─────────────────────────────────────

impl Component for Settings {
    type Message = SettingsMessage;
    type Event = SettingsEvent;

    fn update(
        &mut self,
        message: SettingsMessage,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        match message {
            SettingsMessage::Close => {
                return (Task::none(), Some(SettingsEvent::Closed));
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
                self.open_select = None;
                return (Task::none(), Some(SettingsEvent::DateDisplayChanged(self.date_display)));
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
                self.overlay = None;
                self.overlay_anim.go_mut(false, Instant::now());
                return (Task::none(), None);
            }
            SettingsMessage::SaveAccountEditor => {
                return self.handle_account_editor_save();
            }
            SettingsMessage::DeleteAccountRequested(id) => {
                if let Some(ref mut editor) = self.editing_account {
                    if editor.account_id == id {
                        editor.show_delete_confirmation = true;
                    }
                }
                return (Task::none(), None);
            }
            SettingsMessage::DeleteAccountConfirmed(id) => {
                self.editing_account = None;
                self.overlay = None;
                self.overlay_anim.go_mut(false, Instant::now());
                return (Task::none(), Some(SettingsEvent::DeleteAccount(id)));
            }
            SettingsMessage::DeleteAccountCancelled => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.show_delete_confirmation = false;
                }
                return (Task::none(), None);
            }
            SettingsMessage::ReauthenticateAccount(_id) => {
                // TODO: Wire to re-auth flow (Phase 7)
                return (Task::none(), None);
            }
            SettingsMessage::SignatureEditorSave => {
                return self.handle_signature_save();
            }
            SettingsMessage::SignatureDelete(id) => {
                return (Task::none(), Some(SettingsEvent::DeleteSignature(id)));
            }
            SettingsMessage::ListDragMove(list_id, point) => {
                return (self.handle_drag_move(list_id, point), None);
            }
            SettingsMessage::SelectTab(Tab::People) => {
                self.active_tab = Tab::People;
                self.pinned_help = None;
                // LoadContacts handler in main.rs also loads groups.
                return (Task::none(), Some(SettingsEvent::LoadContacts(self.contact_filter.clone())));
            }
            SettingsMessage::ContactEditorSave => {
                return self.handle_contact_save();
            }
            SettingsMessage::ContactDelete(id) => {
                return self.handle_contact_delete(id);
            }
            SettingsMessage::GroupEditorSave => {
                return self.handle_group_save();
            }
            SettingsMessage::GroupDelete(id) => {
                return self.handle_group_delete(id);
            }
            SettingsMessage::ContactFilterChanged(v) => {
                self.contact_filter = v.clone();
                return (Task::none(), Some(SettingsEvent::LoadContacts(v)));
            }
            SettingsMessage::GroupFilterChanged(v) => {
                self.group_filter = v.clone();
                return (Task::none(), Some(SettingsEvent::LoadGroups(v)));
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
            | SettingsMessage::OverlayAnimTick(_) => {}
            SettingsMessage::UndoInput(field) => { self.undo_field(field); }
            SettingsMessage::RedoInput(field) => { self.redo_field(field); }
            SettingsMessage::HelpHover(id) => self.hovered_help = Some(id),
            SettingsMessage::HelpUnhover(id) => {
                if self.hovered_help.as_ref() == Some(&id) {
                    self.hovered_help = None;
                }
            }
            SettingsMessage::ToggleHelpPin(id) => {
                if self.pinned_help.as_ref() == Some(&id) {
                    self.pinned_help = None;
                } else {
                    self.pinned_help = Some(id);
                }
            }
            SettingsMessage::DismissHelp => {
                self.pinned_help = None;
                self.hovered_help = None;
            }
            SettingsMessage::SelectTab(tab) => {
                self.active_tab = tab;
                self.pinned_help = None;
            }
            SettingsMessage::ToggleSelect(field) => {
                self.open_select = if self.open_select == Some(field) { None } else { Some(field) };
            }
            SettingsMessage::ScaleDragged(v) => self.scale_preview = Some(v),
            SettingsMessage::ScaleReleased => {
                if let Some(v) = self.scale_preview.take() { self.scale = v; }
            }
            SettingsMessage::ThemeChanged(v) => { self.theme = v; self.open_select = None; }
            SettingsMessage::DensityChanged(v) => { self.density = v; self.open_select = None; }
            SettingsMessage::FontSizeChanged(v) => { self.font_size = v; self.open_select = None; }
            SettingsMessage::ReadingPaneChanged(v) => { self.reading_pane_position = v; self.open_select = None; }
            SettingsMessage::ThemeSelected(i) => { self.selected_theme = Some(i); self.theme = "Theme".into(); }
            SettingsMessage::ToggleSyncStatusBar(v) => self.sync_status_bar = v,
            SettingsMessage::ToggleBlockRemoteImages(v) => self.block_remote_images = v,
            SettingsMessage::TogglePhishingDetection(v) => self.phishing_detection = v,
            SettingsMessage::PhishingSensitivityChanged(v) => self.phishing_sensitivity = v,
            SettingsMessage::ToggleSendAndArchive(v) => self.send_and_archive = v,
            SettingsMessage::UndoDelayChanged(v) => { self.undo_delay = v; self.open_select = None; }
            SettingsMessage::DefaultReplyChanged(v) => { self.default_reply_mode = v; self.open_select = None; }
            SettingsMessage::MarkAsReadChanged(v) => { self.mark_as_read = v; self.open_select = None; }
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
            SettingsMessage::AiProviderChanged(v) => { self.ai_provider = v; self.open_select = None; }
            SettingsMessage::AiModelChanged(v) => { self.ai_model = v; self.open_select = None; }
            SettingsMessage::ToggleAiEnabled(v) => self.ai_enabled = v,
            SettingsMessage::ToggleAiAutoCategorize(v) => self.ai_auto_categorize = v,
            SettingsMessage::ToggleAiAutoSummarize(v) => self.ai_auto_summarize = v,
            SettingsMessage::ToggleAiAutoDraft(v) => self.ai_auto_draft = v,
            SettingsMessage::ToggleAiWritingStyle(v) => self.ai_writing_style = v,
            SettingsMessage::ToggleAiAutoArchiveUpdates(v) => self.ai_auto_archive_updates = v,
            SettingsMessage::ToggleAiAutoArchivePromotions(v) => self.ai_auto_archive_promotions = v,
            SettingsMessage::ToggleAiAutoArchiveSocial(v) => self.ai_auto_archive_social = v,
            SettingsMessage::ToggleAiAutoArchiveNewsletters(v) => self.ai_auto_archive_newsletters = v,
            SettingsMessage::AiApiKeyChanged(v) => self.ai_api_key.set_text(v),
            SettingsMessage::OllamaUrlChanged(v) => self.ai_ollama_url.set_text(v),
            SettingsMessage::OllamaModelChanged(v) => self.ai_ollama_model.set_text(v),
            SettingsMessage::SaveAiSettings => self.ai_key_saved = true,
            SettingsMessage::ListGripPress(list_id, index) => {
                self.drag_state = Some(DragState {
                    list_id, dragging_index: index, start_y: -1.0, is_dragging: false,
                });
            }
            SettingsMessage::ListDragEnd(_) => self.drag_state = None,
            SettingsMessage::ListRowClick(list_id, index) => {
                if self.drag_state.is_none() {
                    let items = self.list_items_mut(&list_id);
                    if let Some(item) = items.get_mut(index)
                        && let Some(ref mut enabled) = item.enabled
                    {
                        *enabled = !*enabled;
                    }
                }
            }
            SettingsMessage::ListRemove(list_id, index) => {
                let items = self.list_items_mut(&list_id);
                if index < items.len() { items.remove(index); }
            }
            SettingsMessage::ListAdd(list_id) => {
                let items = self.list_items_mut(&list_id);
                items.push(EditableItem { label: format!("New item {}", items.len() + 1), enabled: None });
            }
            SettingsMessage::ListToggle(list_id, index, value) => {
                if let Some(item) = self.list_items_mut(&list_id).get_mut(index) {
                    item.enabled = Some(value);
                }
            }
            SettingsMessage::ListMenu(_, _) => {}
            SettingsMessage::AccountNameEditorChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.account_name = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::DisplayNameEditorChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.display_name = v;
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
                    editor.caldav_url = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::CaldavUsernameChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_username = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::CaldavPasswordChanged(v) => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_password = v;
                    editor.dirty = true;
                }
            }
            SettingsMessage::SignatureEdit(sig_id) => {
                if let Some(sig) = self.signatures.iter().find(|s| s.id == sig_id) {
                    self.signature_editor = Some(SignatureEditorState {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                        name: UndoableText::with_initial(&sig.name),
                        body: UndoableText::with_initial(&sig.body_html),
                        is_default: sig.is_default,
                        is_reply_default: sig.is_reply_default,
                    });
                    self.overlay = Some(SettingsOverlay::EditSignature {
                        signature_id: Some(sig.id.clone()),
                        account_id: sig.account_id.clone(),
                    });
                    self.overlay_anim.go_mut(true, Instant::now());
                }
            }
            SettingsMessage::SignatureCreate(account_id) => {
                self.signature_editor = Some(SignatureEditorState {
                    signature_id: None,
                    account_id: account_id.clone(),
                    name: UndoableText::new(),
                    body: UndoableText::new(),
                    is_default: false,
                    is_reply_default: false,
                });
                self.overlay = Some(SettingsOverlay::EditSignature {
                    signature_id: None,
                    account_id,
                });
                self.overlay_anim.go_mut(true, Instant::now());
            }
            SettingsMessage::SignatureEditorNameChanged(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.name.set_text(v);
                }
            }
            SettingsMessage::SignatureEditorBodyChanged(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.body.set_text(v);
                }
            }
            SettingsMessage::SignatureEditorToggleDefault(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.is_default = v;
                }
            }
            SettingsMessage::SignatureEditorToggleReplyDefault(v) => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.is_reply_default = v;
                }
            }
            SettingsMessage::OpenOverlay(overlay) => {
                self.overlay = Some(overlay);
                self.overlay_anim.go_mut(true, Instant::now());
            }
            SettingsMessage::CloseOverlay => {
                self.overlay = None;
                self.overlay_anim.go_mut(false, Instant::now());
                self.signature_editor = None;
                self.editing_account = None;
                self.contact_editor = None;
                self.group_editor = None;
            }
            SettingsMessage::ContactsLoaded(Ok(contacts)) => {
                self.contacts = contacts;
            }
            SettingsMessage::ContactsLoaded(Err(e)) => {
                eprintln!("Failed to load contacts: {e}");
            }
            SettingsMessage::GroupsLoaded(Ok(groups)) => {
                self.groups = groups;
            }
            SettingsMessage::GroupsLoaded(Err(e)) => {
                eprintln!("Failed to load groups: {e}");
            }
            SettingsMessage::GroupMembersLoaded(group_id, Ok(members)) => {
                if let Some(ref mut editor) = self.group_editor {
                    if editor.group_id.as_deref() == Some(group_id.as_str()) {
                        editor.members = members;
                    }
                }
            }
            SettingsMessage::GroupMembersLoaded(_, Err(e)) => {
                eprintln!("Failed to load group members: {e}");
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
                        ContactField::DisplayName => editor.display_name = value,
                        ContactField::Email => editor.email = value,
                        ContactField::Email2 => editor.email2 = value,
                        ContactField::Phone => editor.phone = value,
                        ContactField::Company => editor.company = value,
                        ContactField::Notes => editor.notes = value,
                    }
                }
            }
            SettingsMessage::ContactSaved(Ok(())) | SettingsMessage::ContactDeleted(Ok(())) => {}
            SettingsMessage::ContactSaved(Err(_)) | SettingsMessage::ContactDeleted(Err(_)) => {}
            SettingsMessage::GroupCreate => {
                self.open_new_group_editor();
            }
            SettingsMessage::GroupEditorNameChanged(v) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.name = v;
                }
            }
            SettingsMessage::GroupEditorRemoveMember(email) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.members.retain(|m| m != &email);
                }
            }
            SettingsMessage::GroupEditorAddMember(email) => {
                if let Some(ref mut editor) = self.group_editor {
                    if !editor.members.contains(&email) {
                        editor.members.push(email);
                    }
                }
            }
            SettingsMessage::GroupEditorFilterChanged(v) => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.filter = v;
                }
            }
            SettingsMessage::GroupSaved(Ok(())) | SettingsMessage::GroupDeleted(Ok(())) => {}
            SettingsMessage::GroupSaved(Err(_)) | SettingsMessage::GroupDeleted(Err(_)) => {}
            _ => {} // Already handled in update() or handle_simple_message()
        }
    }

    fn handle_drag_move(&mut self, list_id: String, point: Point) -> Task<SettingsMessage> {
        let has_drag = self.drag_state.as_ref().is_some_and(|d| d.list_id == list_id);
        if !has_drag { return Task::none(); }

        if let Some(ref mut drag) = self.drag_state
            && drag.start_y < 0.0
        {
            drag.start_y = point.y;
            return Task::none();
        }

        let Some(drag_ref) = self.drag_state.as_ref() else { return Task::none() };
        let (from, start_y) = (drag_ref.dragging_index, drag_ref.start_y);

        if !drag_ref.is_dragging {
            if (point.y - start_y).abs() < DRAG_START_THRESHOLD { return Task::none(); }
            if let Some(ref mut drag) = self.drag_state { drag.is_dragging = true; }
        }

        let row_step = SETTINGS_ROW_HEIGHT + 1.0;
        let count = self.list_items_mut(&list_id).len();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let target = ((point.y / row_step).max(0.0) as usize).min(count.saturating_sub(1));

        if target != from {
            self.list_items_mut(&list_id).swap(from, target);
            if let Some(ref mut drag) = self.drag_state { drag.dragging_index = target; }
        }
        Task::none()
    }

    fn undo_field(&mut self, field: InputField) {
        match field {
            InputField::VipEmail => { self.vip_email_input.undo(); }
            InputField::AiApiKey => { self.ai_api_key.undo(); }
            InputField::OllamaUrl => { self.ai_ollama_url.undo(); }
            InputField::OllamaModel => { self.ai_ollama_model.undo(); }
            InputField::SignatureName => {
                if let Some(ref mut editor) = self.signature_editor { editor.name.undo(); }
            }
            InputField::SignatureBody => {
                if let Some(ref mut editor) = self.signature_editor { editor.body.undo(); }
            }
        }
    }

    fn redo_field(&mut self, field: InputField) {
        match field {
            InputField::VipEmail => { self.vip_email_input.redo(); }
            InputField::AiApiKey => { self.ai_api_key.redo(); }
            InputField::OllamaUrl => { self.ai_ollama_url.redo(); }
            InputField::OllamaModel => { self.ai_ollama_model.redo(); }
            InputField::SignatureName => {
                if let Some(ref mut editor) = self.signature_editor { editor.name.redo(); }
            }
            InputField::SignatureBody => {
                if let Some(ref mut editor) = self.signature_editor { editor.body.redo(); }
            }
        }
    }

    fn handle_signature_save(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.signature_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.text().trim().to_string();
        if name.is_empty() {
            return (Task::none(), None);
        }
        let request = SignatureSaveRequest {
            id: editor.signature_id.clone(),
            account_id: editor.account_id.clone(),
            name,
            body_html: editor.body.text().to_string(),
            is_default: editor.is_default,
            is_reply_default: editor.is_reply_default,
        };
        // Close overlay
        self.overlay = None;
        self.overlay_anim.go_mut(false, Instant::now());
        self.signature_editor = None;
        (Task::none(), Some(SettingsEvent::SaveSignature(request)))
    }

    fn open_contact_editor(&mut self, contact_id: &str) {
        if let Some(contact) = self.contacts.iter().find(|c| c.id == contact_id) {
            self.contact_editor = Some(ContactEditorState {
                contact_id: Some(contact.id.clone()),
                account_id: contact.account_id.clone(),
                display_name: contact.display_name.clone().unwrap_or_default(),
                email: contact.email.clone(),
                email2: contact.email2.clone().unwrap_or_default(),
                phone: contact.phone.clone().unwrap_or_default(),
                company: contact.company.clone().unwrap_or_default(),
                notes: contact.notes.clone().unwrap_or_default(),
            });
            self.overlay = Some(SettingsOverlay::EditContact {
                contact_id: Some(contact.id.clone()),
            });
            self.overlay_anim.go_mut(true, Instant::now());
        }
    }

    fn open_new_contact_editor(&mut self) {
        self.contact_editor = Some(ContactEditorState {
            contact_id: None,
            account_id: None,
            display_name: String::new(),
            email: String::new(),
            email2: String::new(),
            phone: String::new(),
            company: String::new(),
            notes: String::new(),
        });
        self.overlay = Some(SettingsOverlay::EditContact { contact_id: None });
        self.overlay_anim.go_mut(true, Instant::now());
    }

    fn handle_contact_save(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.contact_editor else {
            return (Task::none(), None);
        };
        let email = editor.email.trim().to_string();
        if email.is_empty() {
            return (Task::none(), None);
        }
        let entry = crate::db::ContactEntry {
            id: editor.contact_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            email,
            display_name: non_empty(editor.display_name.trim()),
            email2: non_empty(editor.email2.trim()),
            phone: non_empty(editor.phone.trim()),
            company: non_empty(editor.company.trim()),
            notes: non_empty(editor.notes.trim()),
            account_id: editor.account_id.clone(),
            account_color: None,
            groups: Vec::new(),
        };
        self.overlay = None;
        self.overlay_anim.go_mut(false, Instant::now());
        self.contact_editor = None;
        (Task::none(), Some(SettingsEvent::SaveContact(entry)))
    }

    fn handle_contact_delete(&mut self, id: String) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        self.overlay = None;
        self.overlay_anim.go_mut(false, Instant::now());
        self.contact_editor = None;
        (Task::none(), Some(SettingsEvent::DeleteContact(id)))
    }

    fn open_group_editor(&mut self, group_id: &str) {
        if let Some(group) = self.groups.iter().find(|g| g.id == group_id) {
            self.group_editor = Some(GroupEditorState {
                group_id: Some(group.id.clone()),
                name: group.name.clone(),
                members: Vec::new(), // will be populated from DB via App
                filter: String::new(),
            });
            self.overlay = Some(SettingsOverlay::EditGroup {
                group_id: Some(group.id.clone()),
            });
            self.overlay_anim.go_mut(true, Instant::now());
        }
    }

    fn open_new_group_editor(&mut self) {
        self.group_editor = Some(GroupEditorState {
            group_id: None,
            name: String::new(),
            members: Vec::new(),
            filter: String::new(),
        });
        self.overlay = Some(SettingsOverlay::EditGroup { group_id: None });
        self.overlay_anim.go_mut(true, Instant::now());
    }

    fn handle_group_save(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.group_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.trim().to_string();
        if name.is_empty() {
            return (Task::none(), None);
        }
        let group = crate::db::GroupEntry {
            id: editor.group_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            member_count: editor.members.len() as i64,
            created_at: 0,
            updated_at: 0,
        };
        let members = editor.members.clone();
        self.overlay = None;
        self.overlay_anim.go_mut(false, Instant::now());
        self.group_editor = None;
        (Task::none(), Some(SettingsEvent::SaveGroup(group, members)))
    }

    fn handle_group_delete(&mut self, id: String) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        self.overlay = None;
        self.overlay_anim.go_mut(false, Instant::now());
        self.group_editor = None;
        (Task::none(), Some(SettingsEvent::DeleteGroup(id)))
    }

    pub(super) fn list_items_mut(&mut self, list_id: &str) -> &mut Vec<EditableItem> {
        match list_id {
            "labels" => &mut self.demo_labels,
            "filters" => &mut self.demo_filters,
            _ => &mut self.demo_labels,
        }
    }

    fn open_account_editor(&mut self, account_id: &str) {
        let Some(account) = self.managed_accounts.iter().find(|a| a.id == account_id) else {
            return;
        };
        // Resolve current color index from hex
        let presets = ratatoskr_label_colors::category_colors::all_presets();
        let color_index = account.account_color.as_deref().and_then(|hex| {
            presets.iter().position(|(_, bg, _)| *bg == hex)
        });

        self.editing_account = Some(AccountEditor {
            account_id: account.id.clone(),
            account_email: account.email.clone(),
            account_name: account.account_name.clone().unwrap_or_default(),
            display_name: account.display_name.clone().unwrap_or_default(),
            account_color_index: color_index,
            caldav_url: String::new(),
            caldav_username: String::new(),
            caldav_password: String::new(),
            show_delete_confirmation: false,
            dirty: false,
        });
        self.overlay = Some(SettingsOverlay::AccountEditor);
        self.overlay_anim.go_mut(true, iced::time::Instant::now());
    }

    fn handle_account_editor_save(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.editing_account else {
            return (Task::none(), None);
        };
        if !editor.dirty {
            // Nothing changed — just close
            self.editing_account = None;
            self.overlay = None;
            self.overlay_anim.go_mut(false, iced::time::Instant::now());
            return (Task::none(), None);
        }

        let presets = ratatoskr_label_colors::category_colors::all_presets();
        let color_hex = editor.account_color_index
            .and_then(|i| presets.get(i))
            .map(|(_, bg, _)| (*bg).to_string());

        let params = ratatoskr_core::db::queries_extra::UpdateAccountParams {
            account_name: Some(editor.account_name.clone()),
            display_name: Some(editor.display_name.clone()),
            account_color: color_hex,
            caldav_url: non_empty(editor.caldav_url.trim()),
            caldav_username: non_empty(editor.caldav_username.trim()),
            caldav_password: non_empty(editor.caldav_password.trim()),
        };
        let account_id = editor.account_id.clone();

        self.editing_account = None;
        self.overlay = None;
        self.overlay_anim.go_mut(false, iced::time::Instant::now());
        (Task::none(), Some(SettingsEvent::SaveAccountChanges {
            account_id,
            params,
        }))
    }
}

/// Convert empty strings to `None`.
fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_string()) }
}
