#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::{CommandContext, CommandMatch, CommandRegistry};

#[tauri::command]
pub fn command_palette_query(
    registry: State<'_, CommandRegistry>,
    ctx: CommandContext,
    query: String,
) -> Vec<CommandMatch> {
    registry.query(&ctx, &query)
}
