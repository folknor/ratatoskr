use std::sync::Arc;

use cmdk::{CommandContext, CommandId, CommandInputResolver, OptionItem};

use crate::db::Db;

/// Concrete `CommandInputResolver` that queries the app's DB for
/// folders, labels, and accounts to populate the palette's stage 2
/// option lists.
pub struct AppInputResolver {
    db: Arc<Db>,
}

impl AppInputResolver {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

impl CommandInputResolver for AppInputResolver {
    fn get_options(
        &self,
        command_id: CommandId,
        param_index: usize,
        _prior_selections: &[String],
        ctx: &CommandContext,
    ) -> Result<Vec<OptionItem>, String> {
        match (command_id, param_index) {
            (CommandId::EmailMoveToFolder, 0) => {
                let account_id = ctx
                    .active_account_id
                    .as_deref()
                    .ok_or_else(|| "no active account".to_string())?;
                self.db.get_user_folders_for_palette(account_id)
            }
            (CommandId::EmailAddLabel, 0) => {
                let account_id = ctx
                    .active_account_id
                    .as_deref()
                    .ok_or_else(|| "no active account".to_string())?;
                self.db.get_user_labels_for_palette(account_id)
            }
            (CommandId::EmailRemoveLabel, 0) => {
                let account_id = ctx
                    .active_account_id
                    .as_deref()
                    .ok_or_else(|| "no active account".to_string())?;
                let thread_id = ctx
                    .selected_thread_ids
                    .first()
                    .ok_or_else(|| "no thread selected".to_string())?;
                self.db.get_thread_labels_for_palette(account_id, thread_id)
            }
            (CommandId::NavigateToLabel, 0) => self.db.get_all_labels_cross_account(),
            _ => Ok(vec![]),
        }
    }

    fn validate_option(
        &self,
        _command_id: CommandId,
        _param_index: usize,
        _value: &str,
        _prior_selections: &[String],
        _ctx: &CommandContext,
    ) -> Result<(), String> {
        // Lenient for now — accept any value.
        Ok(())
    }
}
