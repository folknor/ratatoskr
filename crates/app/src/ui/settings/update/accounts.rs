use iced::time::Instant;
use iced::{Point, Task};

use crate::ui::layout::*;
use crate::ui::settings::types::*;
use crate::ui::undoable::UndoableText;

use super::helpers::non_empty;

impl Settings {
    pub(super) fn handle_account_drag_move(
        &mut self,
        point: Point,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        if self.account_drag.is_none() {
            return (Task::none(), None);
        }

        if let Some(ref mut drag) = self.account_drag
            && drag.start_y < 0.0
        {
            drag.start_y = point.y;
            return (Task::none(), None);
        }

        let Some(drag_ref) = self.account_drag.as_ref() else {
            return (Task::none(), None);
        };
        let (from, start_y) = (drag_ref.dragging_index, drag_ref.start_y);

        if !drag_ref.is_dragging {
            if (point.y - start_y).abs() < DRAG_START_THRESHOLD {
                return (Task::none(), None);
            }
            if let Some(ref mut drag) = self.account_drag {
                drag.is_dragging = true;
            }
        }

        let row_step = SETTINGS_TOGGLE_ROW_HEIGHT + 1.0;
        let count = self.managed_accounts.len();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let target = ((point.y / row_step).max(0.0) as usize).min(count.saturating_sub(1));

        if target != from {
            self.managed_accounts.swap(from, target);
            if let Some(ref mut drag) = self.account_drag {
                drag.dragging_index = target;
            }
        }
        (Task::none(), None)
    }

    pub(super) fn handle_account_drag_end(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let was_dragging = self.account_drag.as_ref().is_some_and(|d| d.is_dragging);
        self.account_drag = None;

        if was_dragging {
            let orders: Vec<(String, i64)> = self
                .managed_accounts
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    #[allow(clippy::cast_possible_wrap)]
                    (a.id.clone(), i as i64)
                })
                .collect();
            return (Task::none(), Some(SettingsEvent::ReorderAccounts(orders)));
        }
        (Task::none(), None)
    }

    pub(super) fn open_account_editor(&mut self, account_id: &str) {
        let Some(account) = self.managed_accounts.iter().find(|a| a.id == account_id) else {
            return;
        };
        let presets = label_colors::preset_colors::all_presets();
        // Saved hex strings may not be a preset hex literally (dev-seed
        // accounts use Google brand colors, older accounts may have arbitrary
        // user-picked hex). Snap to the nearest preset so the picker still
        // shows a selected swatch and the user can confirm or change it.
        let color_index = account
            .account_color
            .as_deref()
            .and_then(label_colors::preset_colors::nearest_exchange_preset)
            .and_then(|name| presets.iter().position(|(n, _, _)| *n == name));

        self.editing_account = Some(AccountEditor {
            account_id: account.id.clone(),
            account_email: account.email.clone(),
            account_name: UndoableText::with_initial(account.account_name.as_deref().unwrap_or("")),
            display_name: UndoableText::with_initial(account.display_name.as_deref().unwrap_or("")),
            account_color_index: color_index,
            caldav_url: UndoableText::new(),
            caldav_username: UndoableText::new(),
            caldav_password: UndoableText::new(),
            show_delete_confirmation: false,
            dirty: false,
        });
        self.active_sheet = Some(SettingsSheetPage::AccountEditor);
        self.sheet_anim.go_mut(true, Instant::now());
    }

    pub(super) fn handle_account_editor_save(
        &mut self,
    ) -> (Task<SettingsMessage>, Option<SettingsEvent>) {
        let Some(ref editor) = self.editing_account else {
            return (Task::none(), None);
        };
        if !editor.dirty {
            self.editing_account = None;
            self.active_sheet = None;
            self.sheet_anim.go_mut(false, Instant::now());
            return (Task::none(), None);
        }

        let presets = label_colors::preset_colors::all_presets();
        let color_hex = editor
            .account_color_index
            .and_then(|i| presets.get(i))
            .map(|(_, bg, _)| (*bg).to_string());

        let params = rtsk::db::queries_extra::UpdateAccountParams {
            account_name: Some(editor.account_name.text().to_string()),
            display_name: Some(editor.display_name.text().to_string()),
            account_color: color_hex,
            caldav_url: non_empty(editor.caldav_url.text().trim()),
            caldav_username: non_empty(editor.caldav_username.text().trim()),
            caldav_password: non_empty(editor.caldav_password.text().trim()),
            // Attachments roadmap Phase 6: the per-account toggle has
            // no UI widget yet (user is implementing settings UI
            // separately). Leave None so this editor save doesn't
            // overwrite whatever the (future) toggle wrote.
            cache_attachments_enabled: None,
        };
        let account_id = editor.account_id.clone();

        self.editing_account = None;
        self.active_sheet = None;
        self.sheet_anim.go_mut(false, Instant::now());
        (
            Task::none(),
            Some(SettingsEvent::SaveAccountChanges { account_id, params }),
        )
    }
}
