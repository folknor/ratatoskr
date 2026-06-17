use iced::Task;
use iced::time::Instant;

use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn handle_signature_save(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.signature_editor else {
            return (Task::none(), None);
        };
        let name = editor.name.text().trim().to_string();
        if name.is_empty() || editor.account_id.is_empty() {
            return (Task::none(), None);
        }
        let request = SignatureSaveRequest {
            id: editor.signature_id.clone(),
            account_id: editor.account_id.clone(),
            name,
            body_html: editor.body_editor.to_html(),
            is_default: editor.is_default,
            is_reply_default: editor.is_reply_default,
        };
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        self.signature_editor = None;
        (Task::none(), Some(SettingsEvent::SaveSignature(request)))
    }
}
