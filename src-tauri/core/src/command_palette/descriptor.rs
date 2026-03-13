use serde::Serialize;

use super::context::CommandContext;
use super::id::CommandId;

pub struct CommandDescriptor {
    pub id: CommandId,
    pub label: &'static str,
    pub category: &'static str,
    pub keybinding: Option<&'static str>,
    pub active_label: Option<&'static str>,
    pub is_available: fn(&CommandContext) -> bool,
    pub is_active: Option<fn(&CommandContext) -> bool>,
}

impl CommandDescriptor {
    pub fn resolved_label(&self, ctx: &CommandContext) -> &'static str {
        if let Some(is_active) = self.is_active
            && let Some(active_label) = self.active_label
            && is_active(ctx)
        {
            return active_label;
        }
        self.label
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandMatch {
    pub id: CommandId,
    pub label: &'static str,
    pub category: &'static str,
    pub keybinding: Option<&'static str>,
    pub available: bool,
    pub score: u32,
    pub match_positions: Vec<u32>,
}
