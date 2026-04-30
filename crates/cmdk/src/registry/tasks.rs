use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::keybinding::KeyBinding;

use super::builders::{desc, with_docs};
use super::scoring::{always, needs_selection};

pub(super) fn register_tasks(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc(CommandId::TaskCreate, "New Task", "Tasks", None, always),
        "Create Task",
        "Create a new task.",
    ));
    out.push(with_docs(
        desc(
            CommandId::TaskCreateFromEmail,
            "Task from Email",
            "Tasks",
            Some(KeyBinding::key('t')),
            needs_selection,
        ),
        "Create Task from Email",
        "Create a task linked to the selected email thread.",
    ));
    out.push(with_docs(
        desc(
            CommandId::TaskTogglePanel,
            "Toggle Panel",
            "Tasks",
            None,
            always,
        ),
        "Toggle Task Panel",
        "Show or hide the task panel.",
    ));
    out.push(with_docs(
        desc(CommandId::TaskViewAll, "View All", "Tasks", None, always),
        "View All Tasks",
        "Show the full task list.",
    ));
}
