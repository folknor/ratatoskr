use iced::{Point, Task};

use crate::ui::layout::*;
use crate::ui::settings::types::*;

impl Settings {
    pub(super) fn handle_drag_move(&mut self, list_id: &str, point: Point) -> Task<SettingsMessage> {
        let has_drag = self
            .drag_state
            .as_ref()
            .is_some_and(|d| d.list_id == list_id);
        if !has_drag {
            return Task::none();
        }

        if let Some(ref mut drag) = self.drag_state
            && drag.start_y < 0.0
        {
            drag.start_y = point.y;
            return Task::none();
        }

        let Some(drag_ref) = self.drag_state.as_ref() else {
            return Task::none();
        };
        let (from, start_y) = (drag_ref.dragging_index, drag_ref.start_y);

        if !drag_ref.is_dragging {
            if (point.y - start_y).abs() < DRAG_START_THRESHOLD {
                return Task::none();
            }
            if let Some(ref mut drag) = self.drag_state {
                drag.is_dragging = true;
            }
        }

        let row_step = SETTINGS_ROW_HEIGHT + 1.0;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let raw_target = (point.y / row_step).max(0.0) as usize;

        // Labels list - `list_id` is `labels:{account_id}`.
        if let Some(account_id) = list_id.strip_prefix("labels:") {
            let Some(labels) = self.label_rows_for_account_mut(account_id) else {
                return Task::none();
            };
            let count = labels.len();
            let target = raw_target.min(count.saturating_sub(1));
            if target != from {
                labels.swap(from, target);
                if let Some(ref mut drag) = self.drag_state {
                    drag.dragging_index = target;
                }
            }
            return Task::none();
        }

        let count = self.list_items_mut(list_id).len();
        let target = raw_target.min(count.saturating_sub(1));

        if target != from {
            self.list_items_mut(list_id).swap(from, target);
            if let Some(ref mut drag) = self.drag_state {
                drag.dragging_index = target;
            }
        }
        Task::none()
    }

    pub(super) fn list_items_mut(&mut self, list_id: &str) -> &mut Vec<EditableItem> {
        // Labels live in `labels_by_account` and are handled in
        // `handle_drag_move` via `label_rows_for_account_mut`. This helper
        // only services the `EditableItem`-backed lists.
        match list_id {
            "filters" => &mut self.demo_filters,
            _ => &mut self.demo_filters,
        }
    }

    pub(super) fn label_rows_for_account_mut(
        &mut self,
        account_id: &str,
    ) -> Option<&mut Vec<rtsk::db::queries_extra::navigation::AccountLabelRow>> {
        self.labels_by_account
            .iter_mut()
            .find(|g| g.account_id == account_id)
            .map(|g| &mut g.labels)
    }
}
