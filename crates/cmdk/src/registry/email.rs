use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::input::{InputSchema, ParamDef};
use crate::keybinding::KeyBinding;

use super::builders::{desc, desc_kw, parameterized, toggle, undoable, with_docs, with_keywords};
use super::scoring::{always, needs_selection, needs_single_selection};

pub(super) fn register_email(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        undoable(desc_kw(
            CommandId::EmailArchive,
            "Archive",
            "Email",
            Some(KeyBinding::key('e')),
            |ctx| ctx.has_selection() && ctx.allows_remove_items(),
            &["done", "file"],
        )),
        "Archive",
        "Remove the thread from inbox without deleting it. Can be undone.",
    ));
    out.push(with_docs(
        undoable(desc_kw(
            CommandId::EmailTrash,
            "Delete",
            "Email",
            Some(KeyBinding::key('#')),
            |ctx| {
                ctx.has_selection()
                    && ctx.thread_in_trash != Some(true)
                    && ctx.allows_remove_items()
            },
            &["delete", "remove", "trash"],
        )),
        "Delete - Move to Trash",
        "Move the selected thread to the Trash folder. Can be undone.",
    ));
    out.push(with_docs(desc(
        CommandId::EmailPermanentDelete, "Permanently Delete", "Email", None,
        |ctx| ctx.has_selection() && ctx.thread_in_trash == Some(true) && ctx.allows_remove_items(),
    ), "Permanently Delete", "Permanently delete the thread. This cannot be undone. Only available for threads already in Trash."));
    out.push(with_docs(
        undoable(toggle(
            CommandId::EmailSpam,
            "Spam",
            "Not Spam",
            "Email",
            Some(KeyBinding::key('!')),
            |ctx| ctx.has_selection() && ctx.allows_remove_items(),
            |ctx| ctx.thread_in_spam == Some(true),
        )),
        "Report Spam / Not Spam",
        "Mark the thread as spam, or remove the spam flag if already marked. Can be undone.",
    ));
    register_email_toggles(out);
    register_email_other(out);
}

fn register_email_toggles(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        undoable(with_keywords(
            toggle(
                CommandId::EmailMarkRead,
                "Mark Read",
                "Mark Unread",
                "Email",
                None,
                |ctx| ctx.has_selection() && ctx.allows_set_seen(),
                |ctx| ctx.thread_is_read == Some(true),
            ),
            &["seen"],
        )),
        "Mark as Read / Unread",
        "Toggle the read/unread status of the selected thread. Can be undone.",
    ));
    out.push(with_docs(undoable(toggle(
        CommandId::EmailStar, "Star", "Unstar", "Email",
        Some(KeyBinding::key('s')),
        |ctx| ctx.has_selection() && ctx.allows_set_keywords(),
        |ctx| ctx.thread_is_starred == Some(true),
    )), "Star / Unstar", "Toggle the star flag on the selected thread. Starred threads appear in the Starred folder. Can be undone."));
    out.push(with_docs(undoable(toggle(
        CommandId::EmailPin, "Pin", "Unpin", "Email",
        Some(KeyBinding::key('p')),
        |ctx| ctx.has_selection() && ctx.allows_set_keywords(),
        |ctx| ctx.thread_is_pinned == Some(true),
    )), "Pin / Unpin", "Pin the thread to the top of the thread list. Pinned threads stay visible regardless of sort order. Can be undone."));
    out.push(with_docs(
        undoable(toggle(
            CommandId::EmailMute,
            "Mute",
            "Unmute",
            "Email",
            Some(KeyBinding::key('m')),
            |ctx| ctx.has_selection() && ctx.allows_set_keywords(),
            |ctx| ctx.thread_is_muted == Some(true),
        )),
        "Mute / Unmute",
        "Mute the thread so new replies don't bring it back to the inbox. Can be undone.",
    ));
}

fn register_email_other(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(desc(
        CommandId::EmailUnsubscribe, "Unsubscribe", "Email",
        Some(KeyBinding::key('u')), needs_single_selection,
    ), "Unsubscribe", "Unsubscribe from the mailing list associated with this thread, if a List-Unsubscribe header is present."));
    out.push(with_docs(
        undoable(with_keywords(
            parameterized(
                CommandId::EmailMoveToFolder,
                "Move",
                "Email",
                Some(KeyBinding::key('v')),
                |ctx| ctx.has_selection() && ctx.allows_remove_items(),
                InputSchema::Single {
                    param: ParamDef::ListPicker { label: "Folder" },
                },
            ),
            &["file", "organize", "move"],
        )),
        "Move to Folder",
        "Move the selected thread to a different folder. Can be undone.",
    ));
    out.push(with_docs(
        undoable(parameterized(
            CommandId::EmailAddLabel,
            "Add Label",
            "Email",
            None,
            needs_selection,
            InputSchema::Single {
                param: ParamDef::ListPicker { label: "Label" },
            },
        )),
        "Add Label",
        "Apply a label group to the selected thread.",
    ));
    out.push(with_docs(
        undoable(parameterized(
            CommandId::EmailRemoveLabel,
            "Remove Label",
            "Email",
            None,
            needs_selection,
            InputSchema::Single {
                param: ParamDef::ListPicker { label: "Label" },
            },
        )),
        "Remove Label",
        "Remove a label group from the selected thread.",
    ));
    out.push(with_docs(
        undoable(parameterized(
            CommandId::EmailSnooze,
            "Snooze",
            "Email",
            None,
            needs_selection,
            InputSchema::Single {
                param: ParamDef::DateTime {
                    label: "Snooze until",
                },
            },
        )),
        "Snooze",
        "Hide the thread until a specified date and time, then return it to the inbox.",
    ));
    out.push(with_docs(
        desc(
            CommandId::EmailSelectAll,
            "Select All",
            "Email",
            Some(KeyBinding::cmd_or_ctrl('a')),
            always,
        ),
        "Select All Threads",
        "Select all threads in the current view.",
    ));
    out.push(with_docs(
        desc(
            CommandId::EmailSelectFromHere,
            "Select From Here",
            "Email",
            Some(KeyBinding::cmd_or_ctrl_shift('a')),
            needs_selection,
        ),
        "Select All From Here",
        "Extend the selection from the current thread to the end of the list.",
    ));
}
