use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::input::{InputSchema, ParamDef};
use crate::keybinding::{KeyBinding, NamedKey};

use super::builders::{desc, parameterized, with_docs};
use super::scoring::{always, needs_selection};

pub(super) fn register_navigation(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc(
            CommandId::NavNext,
            "Next",
            "Navigation",
            Some(KeyBinding::key('j')),
            always,
        ),
        "Next Thread",
        "Move selection to the next thread in the list.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavPrev,
            "Previous",
            "Navigation",
            Some(KeyBinding::key('k')),
            always,
        ),
        "Previous Thread",
        "Move selection to the previous thread in the list.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavOpen,
            "Open",
            "Navigation",
            Some(KeyBinding::key('o')),
            needs_selection,
        ),
        "Open Thread",
        "Open the selected thread in the reading pane.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavMsgNext,
            "Next Message",
            "Navigation",
            Some(KeyBinding::named(NamedKey::ArrowDown)),
            |ctx| ctx.active_message_id.is_some(),
        ),
        "Next Message",
        "Expand and scroll to the next message within the open thread.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavMsgPrev,
            "Previous Message",
            "Navigation",
            Some(KeyBinding::named(NamedKey::ArrowUp)),
            |ctx| ctx.active_message_id.is_some(),
        ),
        "Previous Message",
        "Expand and scroll to the previous message within the open thread.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoInbox,
            "Inbox",
            "Navigation",
            Some(KeyBinding::seq('g', 'i')),
            always,
        ),
        "Go to Inbox",
        "Navigate to the Inbox folder across all accounts.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoStarred,
            "Starred",
            "Navigation",
            Some(KeyBinding::seq('g', 's')),
            always,
        ),
        "Go to Starred",
        "Navigate to the Starred virtual folder.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoSent,
            "Sent",
            "Navigation",
            Some(KeyBinding::seq('g', 't')),
            always,
        ),
        "Go to Sent",
        "Navigate to the Sent folder.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoDrafts,
            "Drafts",
            "Navigation",
            Some(KeyBinding::seq('g', 'd')),
            always,
        ),
        "Go to Drafts",
        "Navigate to the Drafts folder.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoSnoozed,
            "Snoozed",
            "Navigation",
            None,
            always,
        ),
        "Go to Snoozed",
        "Navigate to the Snoozed virtual folder.",
    ));
    out.push(with_docs(
        desc(CommandId::NavGoTrash, "Trash", "Navigation", None, always),
        "Go to Trash",
        "Navigate to the Trash folder.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoAllMail,
            "All Mail",
            "Navigation",
            None,
            always,
        ),
        "Go to All Mail",
        "Navigate to All Mail (all messages across all folders).",
    ));
    out.push(with_docs(
        parameterized(
            CommandId::NavigateToLabel,
            "Go to Label",
            "Navigation",
            Some(KeyBinding::seq('g', 'l')),
            always,
            InputSchema::Single {
                param: ParamDef::ListPicker { label: "Label" },
            },
        ),
        "Go to Label",
        "Navigate to a specific label group by name.",
    ));
    register_navigation_categories(out);
}

fn register_navigation_categories(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        desc(
            CommandId::NavGoPrimary,
            "Primary",
            "Navigation",
            Some(KeyBinding::seq('g', 'p')),
            always,
        ),
        "Go to Primary",
        "Navigate to the Primary category (Gmail) or Focused inbox.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoUpdates,
            "Updates",
            "Navigation",
            Some(KeyBinding::seq('g', 'u')),
            always,
        ),
        "Go to Updates",
        "Navigate to the Updates category.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoPromotions,
            "Promotions",
            "Navigation",
            Some(KeyBinding::seq('g', 'o')),
            always,
        ),
        "Go to Promotions",
        "Navigate to the Promotions category.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoSocial,
            "Social",
            "Navigation",
            Some(KeyBinding::seq('g', 'c')),
            always,
        ),
        "Go to Social",
        "Navigate to the Social category.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoNewsletters,
            "Newsletters",
            "Navigation",
            Some(KeyBinding::seq('g', 'n')),
            always,
        ),
        "Go to Newsletters",
        "Navigate to the Newsletters category.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoTasks,
            "Tasks",
            "Navigation",
            Some(KeyBinding::seq('g', 'k')),
            always,
        ),
        "Go to Tasks",
        "Navigate to the Tasks view.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavGoAttachments,
            "Attachments",
            "Navigation",
            Some(KeyBinding::seq('g', 'a')),
            always,
        ),
        "Go to Attachments",
        "Navigate to the Attachments view.",
    ));
    out.push(with_docs(
        desc(
            CommandId::NavEscape,
            "Close",
            "Navigation",
            Some(KeyBinding::named(NamedKey::Escape)),
            |ctx| ctx.has_selection() || ctx.composer_is_open,
        ),
        "Close / Go Back",
        "Close the current overlay, deselect the thread, or dismiss the composer.",
    ));
}
