use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::keybinding::KeyBinding;

use super::builders::{desc, with_docs};
use super::scoring::always;

pub(super) fn register_view(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc(
            CommandId::ViewToggleSidebar,
            "Toggle Sidebar",
            "View",
            Some(KeyBinding::cmd_or_ctrl_shift('e')),
            always,
        ),
        "Toggle Sidebar",
        "Show or hide the left sidebar.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewSetThemeLight,
            "Light Theme",
            "View",
            None,
            always,
        ),
        "Light Theme",
        "Switch to the light color theme.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewSetThemeDark,
            "Dark Theme",
            "View",
            None,
            always,
        ),
        "Dark Theme",
        "Switch to the dark color theme.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewSetThemeSystem,
            "System Theme",
            "View",
            None,
            always,
        ),
        "System Theme",
        "Follow the operating system's light/dark preference.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewToggleTaskPanel,
            "Toggle Task Panel",
            "View",
            None,
            always,
        ),
        "Toggle Task Panel",
        "Show or hide the task panel.",
    ));
    register_view_reading_pane(out);
}

fn register_view_reading_pane(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc(
            CommandId::ViewReadingPaneRight,
            "Pane Right",
            "View",
            None,
            always,
        ),
        "Reading Pane Right",
        "Position the reading pane to the right of the thread list.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewReadingPaneBottom,
            "Pane Bottom",
            "View",
            None,
            always,
        ),
        "Reading Pane Bottom",
        "Position the reading pane below the thread list.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ViewReadingPaneHidden,
            "Pane Hidden",
            "View",
            None,
            always,
        ),
        "Reading Pane Hidden",
        "Hide the reading pane. Double-click a thread to open it.",
    ));
}
