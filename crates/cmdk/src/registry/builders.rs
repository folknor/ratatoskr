use crate::context::CommandContext;
use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::input::InputSchema;
use crate::keybinding::KeyBinding;

pub(super) fn desc(
    id: CommandId,
    label: &'static str,
    category: &'static str,
    keybinding: Option<KeyBinding>,
    is_available: fn(&CommandContext) -> bool,
) -> CommandDescriptor {
    CommandDescriptor {
        id,
        label,
        category,
        keybinding,
        active_label: None,
        is_available,
        is_active: None,
        input_schema: None,
        keywords: &[],
        is_undoable: false,
        palette_label: None,
        description: None,
    }
}

pub(super) fn desc_kw(
    id: CommandId,
    label: &'static str,
    category: &'static str,
    keybinding: Option<KeyBinding>,
    is_available: fn(&CommandContext) -> bool,
    keywords: &'static [&'static str],
) -> CommandDescriptor {
    CommandDescriptor {
        id,
        label,
        category,
        keybinding,
        active_label: None,
        is_available,
        is_active: None,
        input_schema: None,
        keywords,
        is_undoable: false,
        palette_label: None,
        description: None,
    }
}

pub(super) fn toggle(
    id: CommandId,
    label: &'static str,
    active_label: &'static str,
    category: &'static str,
    keybinding: Option<KeyBinding>,
    is_available: fn(&CommandContext) -> bool,
    is_active: fn(&CommandContext) -> bool,
) -> CommandDescriptor {
    CommandDescriptor {
        id,
        label,
        category,
        keybinding,
        active_label: Some(active_label),
        is_available,
        is_active: Some(is_active),
        input_schema: None,
        keywords: &[],
        is_undoable: false,
        palette_label: None,
        description: None,
    }
}

pub(super) fn undoable(mut d: CommandDescriptor) -> CommandDescriptor {
    d.is_undoable = true;
    d
}

pub(super) fn with_keywords(
    mut d: CommandDescriptor,
    keywords: &'static [&'static str],
) -> CommandDescriptor {
    d.keywords = keywords;
    d
}

pub(super) fn with_docs(
    mut d: CommandDescriptor,
    palette_label: &'static str,
    description: &'static str,
) -> CommandDescriptor {
    d.palette_label = Some(palette_label);
    d.description = Some(description);
    d
}

pub(super) fn parameterized(
    id: CommandId,
    label: &'static str,
    category: &'static str,
    keybinding: Option<KeyBinding>,
    is_available: fn(&CommandContext) -> bool,
    input_schema: InputSchema,
) -> CommandDescriptor {
    CommandDescriptor {
        id,
        label,
        category,
        keybinding,
        active_label: None,
        is_available,
        is_active: None,
        input_schema: Some(input_schema),
        keywords: &[],
        is_undoable: false,
        palette_label: None,
        description: None,
    }
}
