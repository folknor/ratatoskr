use std::sync::Arc;

use super::{CommandContext, CommandId, CommandInputResolver, OptionItem};

/// Concrete newtype wrapper for Tauri managed state.
///
/// Tauri's `State<'_>` lookup is concrete-type based; storing `Arc<dyn Trait>`
/// directly is brittle. This newtype gives a stable concrete type for
/// `app.manage()` and `State<'_, InputResolverState>`.
pub struct InputResolverState(pub Arc<dyn CommandInputResolver>);

/// Stub resolver — returns empty results for all commands.
///
/// Real implementation (querying `DbState` for folders, labels, accounts)
/// is future work. The trait boundary is what matters for this slice.
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
