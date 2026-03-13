#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::resolver::InputResolverState;
use super::{search_options, CommandContext, CommandId, CommandMatch, CommandRegistry, OptionMatch};

#[tauri::command]
pub fn command_palette_query(
    registry: State<'_, CommandRegistry>,
    ctx: CommandContext,
    query: String,
) -> Vec<CommandMatch> {
    registry.query(&ctx, &query)
}

#[tauri::command]
pub fn command_palette_get_options(
    resolver: State<'_, InputResolverState>,
    command_id: CommandId,
    param_index: usize,
    prior_selections: Vec<String>,
    query: String,
    ctx: CommandContext,
) -> Result<Vec<OptionMatch>, String> {
    let items = resolver.0.get_options(command_id, param_index, &prior_selections, &ctx)?;
    Ok(search_options(&items, &query))
}

#[tauri::command]
pub fn command_palette_validate_option(
    resolver: State<'_, InputResolverState>,
    command_id: CommandId,
    param_index: usize,
    value: String,
    prior_selections: Vec<String>,
    ctx: CommandContext,
) -> Result<(), String> {
    resolver
        .0
        .validate_option(command_id, param_index, &value, &prior_selections, &ctx)
}
