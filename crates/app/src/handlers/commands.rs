use iced::Task;

use crate::command_dispatch::{self, EmailAction};
use crate::{APP_DATA_DIR, App, Message};
use ratatoskr_command_palette::{CommandArgs, CommandId, KeyBinding, OptionItem};

impl App {
    /// Save keybinding overrides to disk. Call this after any mutation
    /// to `self.binding_table` overrides (`set_override`, `unbind`,
    /// `remove_override`, `reset_all`).
    pub(crate) fn save_keybinding_overrides(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("keybindings.json");
        if let Err(e) = self.binding_table.save_overrides(&path) {
            eprintln!("warning: failed to save keybinding overrides: {e}");
        }
    }

    /// Set a keybinding override and persist to disk.
    /// Returns `Err(conflicting_id)` if the binding conflicts.
    pub(crate) fn set_keybinding(
        &mut self,
        id: CommandId,
        binding: KeyBinding,
    ) -> Result<(), CommandId> {
        self.binding_table.set_override(id, binding)?;
        self.save_keybinding_overrides();
        Ok(())
    }

    /// Unbind a command (explicit, no fallback to default) and persist.
    pub(crate) fn unbind_keybinding(&mut self, id: CommandId) {
        self.binding_table.unbind(id);
        self.save_keybinding_overrides();
    }

    /// Remove a keybinding override (revert to default) and persist.
    pub(crate) fn remove_keybinding_override(&mut self, id: CommandId) {
        self.binding_table.remove_override(id);
        self.save_keybinding_overrides();
    }

    /// Reset all keybinding overrides to defaults and persist.
    pub(crate) fn reset_all_keybindings(&mut self) {
        self.binding_table.reset_all();
        self.save_keybinding_overrides();
    }

    pub(crate) fn handle_execute_command(&mut self, id: CommandId) -> Task<Message> {
        log::debug!("Executing command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_command(id, self) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    pub(crate) fn handle_execute_parameterized(
        &mut self,
        id: CommandId,
        args: CommandArgs,
    ) -> Task<Message> {
        log::debug!("Executing parameterized command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_parameterized(id, args) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    /// Save usage counts to disk.
    fn save_usage_counts(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("command_usage.json");
        let map = self.registry.usage.to_map();
        match serde_json::to_string(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("Failed to save usage counts: {e}");
                }
            }
            Err(e) => log::warn!("Failed to serialize usage counts: {e}"),
        }
    }

    pub(crate) fn handle_email_action(
        &mut self,
        action: EmailAction,
    ) -> Task<Message> {
        let selection_count = self.thread_list.selection_count();

        let confirmation = match &action {
            EmailAction::Archive => Some("Archived"),
            EmailAction::Trash => Some("Moved to Trash"),
            EmailAction::PermanentDelete => Some("Permanently deleted"),
            EmailAction::ToggleSpam => Some("Spam status toggled"),
            EmailAction::ToggleRead => Some("Read status toggled"),
            EmailAction::ToggleStar => Some("Star toggled"),
            EmailAction::TogglePin => Some("Pin toggled"),
            EmailAction::ToggleMute => Some("Mute toggled"),
            EmailAction::Unsubscribe => Some("Unsubscribed"),
            EmailAction::MoveToFolder { .. } => Some("Moved to folder"),
            EmailAction::AddLabel { .. } => Some("Label applied"),
            EmailAction::RemoveLabel { .. } => Some("Label removed"),
            EmailAction::Snooze { .. } => Some("Snoozed"),
        };
        if let Some(msg) = confirmation {
            let display = if selection_count > 1 {
                format!("{msg} ({selection_count} threads)")
            } else {
                msg.to_string()
            };
            self.status_bar.show_confirmation(display);
        }

        // Destructive actions remove the thread from the current view
        // — trigger auto-advance.
        let removes_from_view = matches!(
            action,
            EmailAction::Archive
                | EmailAction::Trash
                | EmailAction::PermanentDelete
                | EmailAction::ToggleSpam
                | EmailAction::MoveToFolder { .. }
                | EmailAction::Snooze { .. }
        );

        if removes_from_view {
            return self.handle_thread_list(
                crate::ui::thread_list::ThreadListMessage::AutoAdvance,
            );
        }

        // Dispatch label apply/remove locally (DB-only, provider sync picks up changes).
        match action {
            EmailAction::AddLabel { label_id } => {
                return self.apply_label_to_selected_threads(&label_id);
            }
            EmailAction::RemoveLabel { label_id } => {
                return self.remove_label_from_selected_threads(&label_id);
            }
            _ => {}
        }

        Task::none()
    }

    /// Apply a label to all selected threads (local DB operation).
    /// For tag-type labels, inserts thread_labels entries.
    /// Provider write-back happens during the next sync cycle.
    fn apply_label_to_selected_threads(&self, label_id: &str) -> Task<Message> {
        let indices = self.thread_list.selected_indices();
        let threads: Vec<(String, String)> = indices
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if threads.is_empty() {
            return Task::none();
        }

        let db = std::sync::Arc::clone(&self.db);
        let lid = label_id.to_string();

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id) in &threads {
                        conn.execute(
                            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
                             VALUES (?1, ?2, ?3)",
                            rusqlite::params![account_id, thread_id, lid],
                        )
                        .map_err(|e| format!("apply label: {e}"))?;
                    }
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to apply label: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Remove a label from all selected threads (local DB operation).
    fn remove_label_from_selected_threads(&self, label_id: &str) -> Task<Message> {
        let indices = self.thread_list.selected_indices();
        let threads: Vec<(String, String)> = indices
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if threads.is_empty() {
            return Task::none();
        }

        let db = std::sync::Arc::clone(&self.db);
        let lid = label_id.to_string();

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    for (account_id, thread_id) in &threads {
                        conn.execute(
                            "DELETE FROM thread_labels \
                             WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
                            rusqlite::params![account_id, thread_id, lid],
                        )
                        .map_err(|e| format!("remove label: {e}"))?;
                    }
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to remove label: {e}");
                }
                Message::Noop
            },
        )
    }
}

/// Build typed `CommandArgs` from the selected option item.
///
/// Maps each parameterized `CommandId` to its corresponding `CommandArgs`
/// variant, extracting the item's ID (and for cross-account commands,
/// splitting the `"account_id:label_id"` encoding).
pub(crate) fn build_command_args(command_id: CommandId, item: &OptionItem) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => Some(CommandArgs::MoveToFolder {
            folder_id: item.id.clone(),
        }),
        CommandId::EmailAddLabel => Some(CommandArgs::AddLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailRemoveLabel => Some(CommandArgs::RemoveLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailSnooze => {
            // DateTime picker returns a stringified unix timestamp
            item.id
                .parse::<i64>()
                .ok()
                .map(|ts| CommandArgs::Snooze { until: ts })
        }
        CommandId::NavigateToLabel => {
            let (account_id, label_id) = split_cross_account_id(&item.id)?;
            Some(CommandArgs::NavigateToLabel {
                label_id,
                account_id,
            })
        }
        _ => None,
    }
}

/// Build `CommandArgs` from free text input for Text-param commands.
pub(crate) fn build_command_args_from_text(
    command_id: CommandId,
    text: &str,
) -> Option<CommandArgs> {
    match command_id {
        CommandId::SmartFolderSave => Some(CommandArgs::SmartFolderSave {
            name: text.to_string(),
        }),
        _ => None,
    }
}

/// Split a cross-account encoded ID ("account_id:label_id") into its parts.
fn split_cross_account_id(encoded: &str) -> Option<(String, String)> {
    let colon_pos = encoded.find(':')?;
    let account_id = encoded[..colon_pos].to_string();
    let label_id = encoded[colon_pos + 1..].to_string();
    if account_id.is_empty() || label_id.is_empty() {
        return None;
    }
    Some((account_id, label_id))
}
