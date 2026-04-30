use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::keybinding::KeyBinding;

use super::builders::{desc, desc_kw, with_docs};
use super::scoring::always;

pub(super) fn register_compose(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc_kw(
            CommandId::ComposeNew,
            "Compose",
            "Compose",
            Some(KeyBinding::key('c')),
            always,
            &["write", "new", "create", "email"],
        ),
        "Compose New Email",
        "Open the compose window to write a new email.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::ComposeReply,
            "Reply",
            "Compose",
            Some(KeyBinding::key('r')),
            |ctx| ctx.has_selection() && ctx.allows_submit(),
            &["respond"],
        ),
        "Reply",
        "Reply to the sender of the selected message.",
    ));
    out.push(with_docs(
        desc(
            CommandId::ComposeReplyAll,
            "Reply All",
            "Compose",
            Some(KeyBinding::key('a')),
            |ctx| ctx.has_selection() && ctx.allows_submit(),
        ),
        "Reply All",
        "Reply to all recipients of the selected message.",
    ));
    out.push(with_docs(
        desc_kw(
            CommandId::ComposeForward,
            "Forward",
            "Compose",
            Some(KeyBinding::key('f')),
            |ctx| ctx.has_selection() && ctx.allows_submit(),
            &["send", "share"],
        ),
        "Forward",
        "Forward the selected message to new recipients.",
    ));
}
