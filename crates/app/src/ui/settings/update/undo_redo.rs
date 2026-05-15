use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn undo_field(&mut self, field: InputField) {
        match field {
            InputField::VipEmail => {
                self.vip_email_input.undo();
            }
            InputField::AiApiKey => {
                self.ai_api_key.undo();
            }
            InputField::OllamaUrl => {
                self.ai_ollama_url.undo();
            }
            InputField::OllamaModel => {
                self.ai_ollama_model.undo();
            }
            InputField::SignatureName => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.name.undo();
                }
            }
            InputField::AccountName => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.account_name.undo();
                }
            }
            InputField::AccountDisplayName => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.display_name.undo();
                }
            }
            InputField::CaldavUrl => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_url.undo();
                }
            }
            InputField::CaldavUsername => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_username.undo();
                }
            }
            InputField::CaldavPassword => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_password.undo();
                }
            }
            InputField::GroupName => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.name.undo();
                }
            }
            InputField::ContactDisplayName => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.display_name.undo();
                }
            }
            InputField::ContactEmail => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.email.undo();
                }
            }
            InputField::ContactEmail2 => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.email2.undo();
                }
            }
            InputField::ContactPhone => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.phone.undo();
                }
            }
            InputField::ContactCompany => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.company.undo();
                }
            }
            InputField::ContactNotes => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.notes.undo();
                }
            }
            // Label editor fields are plain String today (no UndoableTextInput
            // wrapping yet). Undo is a no-op until we upgrade them.
            InputField::LabelName | InputField::LabelColorBg | InputField::LabelColorFg => {}
        }
    }

    pub(super) fn redo_field(&mut self, field: InputField) {
        match field {
            InputField::VipEmail => {
                self.vip_email_input.redo();
            }
            InputField::AiApiKey => {
                self.ai_api_key.redo();
            }
            InputField::OllamaUrl => {
                self.ai_ollama_url.redo();
            }
            InputField::OllamaModel => {
                self.ai_ollama_model.redo();
            }
            InputField::SignatureName => {
                if let Some(ref mut editor) = self.signature_editor {
                    editor.name.redo();
                }
            }
            InputField::AccountName => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.account_name.redo();
                }
            }
            InputField::AccountDisplayName => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.display_name.redo();
                }
            }
            InputField::CaldavUrl => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_url.redo();
                }
            }
            InputField::CaldavUsername => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_username.redo();
                }
            }
            InputField::CaldavPassword => {
                if let Some(ref mut editor) = self.editing_account {
                    editor.caldav_password.redo();
                }
            }
            InputField::GroupName => {
                if let Some(ref mut editor) = self.group_editor {
                    editor.name.redo();
                }
            }
            InputField::ContactDisplayName => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.display_name.redo();
                }
            }
            InputField::ContactEmail => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.email.redo();
                }
            }
            InputField::ContactEmail2 => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.email2.redo();
                }
            }
            InputField::ContactPhone => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.phone.redo();
                }
            }
            InputField::ContactCompany => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.company.redo();
                }
            }
            InputField::ContactNotes => {
                if let Some(ref mut editor) = self.contact_editor {
                    editor.notes.redo();
                }
            }
            InputField::LabelName | InputField::LabelColorBg | InputField::LabelColorFg => {}
        }
    }
}
