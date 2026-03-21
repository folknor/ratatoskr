use iced::Task;

use crate::command_dispatch::{self, EmailAction};
use crate::{App, Message};
use ratatoskr_command_palette::{CommandArgs, CommandId, OptionItem};

impl App {
    pub(crate) fn handle_execute_command(&mut self, id: CommandId) -> Task<Message> {
        self.registry.usage.record_usage(id);
        match command_dispatch::dispatch_command(id, self) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    /// Handle an email action and show a status bar confirmation.
    ///
    /// The actual server-side mutation is not yet implemented — this
    /// wires the confirmation message so the user sees feedback.
    pub(crate) fn handle_email_action(
        &mut self,
        action: EmailAction,
    ) -> Task<Message> {
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
            self.status_bar.show_confirmation(msg.to_string());
        }
        Task::none()
    }

    pub(crate) fn handle_execute_parameterized(
        &mut self,
        id: CommandId,
        args: CommandArgs,
    ) -> Task<Message> {
        self.registry.usage.record_usage(id);
        match command_dispatch::dispatch_parameterized(id, args) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
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
