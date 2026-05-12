use iced::time::Instant;
use iced::Task;

use crate::ui::settings::types::*;
use crate::ui::undoable::UndoableText;

use super::helpers::non_empty;

impl Settings {
    pub(crate) fn open_contact_editor(&mut self, contact_id: &str) {
        if let Some(contact) = self.contacts.iter().find(|c| c.id == contact_id) {
            self.contact_editor = Some(ContactEditorState {
                contact_id: Some(contact.id.clone()),
                account_id: contact.account_id.clone(),
                display_name: UndoableText::with_initial(
                    contact.display_name.as_deref().unwrap_or(""),
                ),
                email: UndoableText::with_initial(&contact.email),
                email2: UndoableText::with_initial(contact.email2.as_deref().unwrap_or("")),
                phone: UndoableText::with_initial(contact.phone.as_deref().unwrap_or("")),
                company: UndoableText::with_initial(contact.company.as_deref().unwrap_or("")),
                notes: UndoableText::with_initial(contact.notes.as_deref().unwrap_or("")),
                source: contact.source.clone(),
                server_id: contact.server_id.clone(),
                dirty: false,
            });
            self.active_sheet = Some(SettingsSheetPage::EditContact {
                contact_id: Some(contact.id.clone()),
            });
            self.sheet_anim.go_mut(true, Instant::now());
        }
    }

    pub(crate) fn open_new_contact_editor(&mut self) {
        self.contact_editor = Some(ContactEditorState {
            contact_id: None,
            account_id: None,
            display_name: UndoableText::new(),
            email: UndoableText::new(),
            email2: UndoableText::new(),
            phone: UndoableText::new(),
            company: UndoableText::new(),
            notes: UndoableText::new(),
            source: None,
            server_id: None,
            dirty: false,
        });
        self.active_sheet = Some(SettingsSheetPage::EditContact { contact_id: None });
        self.sheet_anim.go_mut(true, Instant::now());
    }

    pub(super) fn handle_contact_save(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.contact_editor else {
            return (Task::none(), None);
        };
        let email = editor.email.text().trim().to_string();
        if email.is_empty() {
            return (Task::none(), None);
        }
        let entry = crate::db::ContactEntry {
            id: editor
                .contact_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            email,
            display_name: non_empty(editor.display_name.text().trim()),
            email2: non_empty(editor.email2.text().trim()),
            phone: non_empty(editor.phone.text().trim()),
            company: non_empty(editor.company.text().trim()),
            notes: non_empty(editor.notes.text().trim()),
            account_id: editor.account_id.clone(),
            account_color: None,
            groups: Vec::new(),
            source: editor.source.clone().or_else(|| Some("user".to_string())),
            server_id: editor.server_id.clone(),
        };
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.contact_editor = None;
        (Task::none(), Some(SettingsEvent::SaveContact(entry)))
    }

    pub(super) fn handle_contact_delete(
        &mut self,
        id: String,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.contact_editor = None;
        (Task::none(), Some(SettingsEvent::DeleteContact(id)))
    }

    pub(super) fn open_group_editor(&mut self, group_id: &str) {
        if let Some(group) = self.groups.iter().find(|g| g.id == group_id) {
            self.group_editor = Some(GroupEditorState {
                group_id: Some(group.id.clone()),
                name: UndoableText::with_initial(&group.name),
                members: Vec::new(),
                filter: String::new(),
                members_filter: String::new(),
                dirty: false,
            });
            self.active_sheet = Some(SettingsSheetPage::EditGroup {
                group_id: Some(group.id.clone()),
            });
            self.sheet_anim.go_mut(true, Instant::now());
        }
    }

    pub(super) fn open_new_group_editor(&mut self) {
        self.group_editor = Some(GroupEditorState {
            group_id: None,
            name: UndoableText::new(),
            members: Vec::new(),
            filter: String::new(),
            members_filter: String::new(),
            dirty: false,
        });
        self.active_sheet = Some(SettingsSheetPage::EditGroup { group_id: None });
        self.sheet_anim.go_mut(true, Instant::now());
    }

    pub(super) fn handle_group_save(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.group_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.text().trim().to_string();
        if name.is_empty() {
            return (Task::none(), None);
        }
        #[allow(clippy::cast_possible_wrap)]
        let member_count = editor.members.len() as i64;
        let group = crate::db::GroupEntry {
            id: editor
                .group_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name,
            member_count,
            created_at: 0,
            updated_at: 0,
        };
        let members = editor.members.clone();
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.group_editor = None;
        (Task::none(), Some(SettingsEvent::SaveGroup(group, members)))
    }

    pub(super) fn handle_group_delete(
        &mut self,
        id: String,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.group_editor = None;
        (Task::none(), Some(SettingsEvent::DeleteGroup(id)))
    }

    pub(super) fn handle_import_file_selected(
        &mut self,
        path: String,
        data: Vec<u8>,
    ) -> Task<SettingsMessage> {
        let Some(ref mut wizard) = self.import_wizard else {
            return Task::none();
        };

        let source = match import::ImportSource::detect(path.clone(), data) {
            Ok(source) => source,
            Err(e) => {
                log::error!("Import format detection error: {e}");
                return Task::none();
            }
        };

        let options = import::ImportOptions::default();
        match import::preview_source(&source, options) {
            Ok(import::ImportPreview::Table(preview)) => {
                wizard.mappings = preview
                    .mappings
                    .iter()
                    .map(|m| ImportContactField::from_import_field(m.target_field))
                    .collect();
                wizard.has_header = preview.has_header;
                wizard.preview = Some(import::ImportPreview::Table(preview));
                wizard.source = Some(source);
                wizard.file_path = Some(path);
                wizard.step = ImportStep::Mapping;
            }
            Ok(import::ImportPreview::Contacts(preview)) => {
                wizard.has_header = false;
                wizard.preview = Some(import::ImportPreview::Contacts(preview));
                wizard.source = Some(source);
                wizard.file_path = Some(path);
                wizard.step = ImportStep::VcfPreview;
            }
            Err(e) => {
                log::error!("Import preview error: {e}");
            }
        }

        Task::none()
    }

    pub(super) fn handle_import_toggle_header(&mut self, has_header: bool) -> Task<SettingsMessage> {
        let Some(ref mut wizard) = self.import_wizard else {
            return Task::none();
        };
        wizard.has_header = has_header;

        if let Some(ref source) = wizard.source {
            let mut options = import::ImportOptions::default().with_header(has_header);
            if let Some(import::ImportPreview::Table(table)) = wizard.preview.as_ref() {
                options.sheet_index = table.selected_sheet;
            }
            match import::preview_source(source, options) {
                Ok(import::ImportPreview::Table(preview)) => {
                    wizard.mappings = preview
                        .mappings
                        .iter()
                        .map(|m| ImportContactField::from_import_field(m.target_field))
                        .collect();
                    wizard.preview = Some(import::ImportPreview::Table(preview));
                }
                Ok(import::ImportPreview::Contacts(preview)) => {
                    wizard.preview = Some(import::ImportPreview::Contacts(preview));
                }
                Err(e) => {
                    log::error!("Import header-toggle preview error: {e}");
                }
            }
        }

        Task::none()
    }

    pub(super) fn handle_import_execute(&mut self) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref mut wizard) = self.import_wizard else {
            return (Task::none(), None);
        };

        let Some(ref source) = wizard.source else {
            return (Task::none(), None);
        };
        let mut options = import::ImportOptions::default().with_header(wizard.has_header);
        let mappings: Vec<import::ColumnMapping> =
            if let Some(import::ImportPreview::Table(table)) = wizard.preview.as_ref() {
                options.sheet_index = table.selected_sheet;
                wizard
                    .mappings
                    .iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let header = table.headers.get(i).cloned().unwrap_or_default();
                        import::ColumnMapping {
                            source_index: i,
                            source_column: header,
                            target_field: field.to_import_field(),
                            confidence: import::MappingConfidence::High,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };

        let prepared = match import::prepare_import(source, &mappings, options) {
            Ok(prepared) => prepared,
            Err(e) => {
                log::error!("Import prepare error: {e}");
                return (Task::none(), None);
            }
        };

        wizard.step = ImportStep::Importing;
        let account_id = wizard.account_id.clone();
        let update_existing = wizard.update_existing;

        (
            Task::none(),
            Some(SettingsEvent::ExecuteContactImport {
                prepared,
                account_id,
                update_existing,
            }),
        )
    }
}
