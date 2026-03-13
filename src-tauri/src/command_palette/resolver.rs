use std::sync::Arc;

use super::{CommandContext, CommandId, CommandInputResolver, OptionItem};

/// Concrete newtype wrapper for Tauri managed state.
///
/// Tauri's `State<'_>` lookup is concrete-type based; storing `Arc<dyn Trait>`
/// directly is brittle. This newtype gives a stable concrete type for
/// `app.manage()` and `State<'_, InputResolverState>`.
pub struct InputResolverState(pub Arc<dyn CommandInputResolver>);

/// **Non-functional stub.** Returns empty option lists and accepts any value.
///
/// The four parameterized commands (`EmailMoveToFolder`, `EmailAddLabel`,
/// `EmailRemoveLabel`, `EmailSnooze`) have schema metadata in the registry
/// but will dead-end through this resolver — `get_options` returns nothing,
/// `validate_option` accepts everything. The frontend must NOT present
/// second-stage input UI for these commands until a real resolver replaces
/// this stub.
///
/// Real implementation will query `DbState` for folders, labels, accounts,
/// and validate selections against current state.
pub struct TauriInputResolver;

impl CommandInputResolver for TauriInputResolver {
    fn get_options(
        &self,
        _command_id: CommandId,
        _param_index: usize,
        _prior_selections: &[String],
        _ctx: &CommandContext,
    ) -> Result<Vec<OptionItem>, String> {
        Ok(vec![])
    }

    fn validate_option(
        &self,
        _command_id: CommandId,
        _param_index: usize,
        _value: &str,
        _prior_selections: &[String],
        _ctx: &CommandContext,
    ) -> Result<(), String> {
        Ok(())
    }
}
