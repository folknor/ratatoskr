use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::keybinding::{KeyBinding, NamedKey};

use super::builders::{desc, desc_kw, with_docs};
use super::scoring::always;

pub(super) fn register_app(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc_kw(
            CommandId::AppSearch,
            "Search",
            "App",
            Some(KeyBinding::key('/')),
            always,
            &["find", "ctrl+f"],
        ),
        "Search",
        "Search across all emails. Supports smart folder query syntax.",
    ));
    out.push(with_docs(
        desc(
            CommandId::AppAskAi,
            "Ask AI",
            "App",
            Some(KeyBinding::key('i')),
            always,
        ),
        "Ask AI",
        "Open the AI assistant.",
    ));
    out.push(with_docs(
        desc(
            CommandId::AppHelp,
            "Shortcuts",
            "App",
            Some(KeyBinding::key('?')),
            always,
        ),
        "Keyboard Shortcuts",
        "Show the keyboard shortcuts reference.",
    ));
    out.push(with_docs(
        desc(
            CommandId::AppSyncFolder,
            "Sync",
            "App",
            Some(KeyBinding::named(NamedKey::F5)),
            |ctx| ctx.active_account_id.is_some(),
        ),
        "Sync Current Folder",
        "Trigger an immediate sync of the current folder with the mail provider.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::AppOpenPalette,
            "Command Palette",
            "App",
            Some(KeyBinding::cmd_or_ctrl('k')),
            always,
            &["palette", "commands"],
        ),
        "Command Palette",
        "Open the command palette to search and execute any command.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::Undo,
            "Undo",
            "App",
            Some(KeyBinding::cmd_or_ctrl('z')),
            always,
            &["revert", "undo"],
        ),
        "Undo",
        "Reverse the last undoable action (archive, delete, star, move, etc.).",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::AppRebuildSearchIndex,
            "Rebuild Search Index",
            "App",
            None,
            always,
            &["rebuild", "reindex", "search", "index"],
        ),
        "Rebuild Search Index",
        "Wipe and rebuild the local search index. Search is unavailable while \
         the rebuild runs; progress is shown in the status bar.",
    ));
}
