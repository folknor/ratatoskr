use crate::context::{CommandContext, FocusedRegion, ViewType};
use crate::descriptor::CommandDescriptor;
use crate::input::InputMode;

pub(super) fn input_mode_for(d: &CommandDescriptor) -> InputMode {
    match d.input_schema {
        Some(schema) => InputMode::Parameterized { schema },
        None => InputMode::Direct,
    }
}

pub(super) fn build_command_haystack(category: &str, label: &str, keywords: &[&str]) -> String {
    let mut haystack = format!("{category} > {label}");
    for kw in keywords {
        haystack.push(' ');
        haystack.push_str(kw);
    }
    haystack
}

/// Returns a modest score boost when the command's category aligns with
/// the current view context or focused region.
pub(super) fn context_boost(descriptor: &CommandDescriptor, ctx: &CommandContext) -> u32 {
    let mut boost: u32 = 0;
    boost += category_relevance(descriptor.category, ctx) * 5;
    boost += focused_region_boost(descriptor.category, ctx);
    boost
}

/// Score bonus from how often this command has been used.
///
/// Log-scaled so high-frequency commands don't dominate: 1 use → 16,
/// 7 uses → 32, 31 uses → 48, 127 uses → 64. Stays well under the
/// 1000 availability bonus so an enabled never-used command still
/// outranks a disabled heavily-used one.
pub(super) fn recency_bonus(count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    (32 - count.saturating_add(1).leading_zeros()) * 8
}

/// Returns 0-4 relevance points for how well a category matches the view.
pub(super) fn category_relevance(category: &str, ctx: &CommandContext) -> u32 {
    match category {
        "Email" => email_view_relevance(ctx),
        "Compose" => compose_view_relevance(ctx),
        "Navigation" => 2,
        "Tasks" if ctx.current_view == ViewType::Tasks => 4,
        "Tasks" => 1,
        "View" => 1,
        "App" => 1,
        _ => 0,
    }
}

fn email_view_relevance(ctx: &CommandContext) -> u32 {
    match ctx.current_view {
        ViewType::Inbox
        | ViewType::Starred
        | ViewType::Sent
        | ViewType::Drafts
        | ViewType::Snoozed
        | ViewType::Trash
        | ViewType::Spam
        | ViewType::AllMail
        | ViewType::SidebarItem
        | ViewType::SmartFolder
        | ViewType::Bundle
        | ViewType::Attachments
        | ViewType::Search
        | ViewType::PinnedSearch => 4,
        _ => 1,
    }
}

fn compose_view_relevance(ctx: &CommandContext) -> u32 {
    if ctx.composer_is_open {
        4
    } else {
        match ctx.current_view {
            ViewType::Drafts => 3,
            ViewType::Inbox | ViewType::Sent => 2,
            _ => 1,
        }
    }
}

fn focused_region_boost(category: &str, ctx: &CommandContext) -> u32 {
    match (category, ctx.focused_region) {
        ("Compose", Some(FocusedRegion::Composer)) => 10,
        ("Email", Some(FocusedRegion::ThreadList | FocusedRegion::ReadingPane)) => 10,
        ("Navigation", Some(FocusedRegion::Sidebar)) => 5,
        _ => 0,
    }
}

pub(super) fn always(_ctx: &CommandContext) -> bool {
    true
}

pub(super) fn needs_selection(ctx: &CommandContext) -> bool {
    ctx.has_selection()
}

pub(super) fn needs_single_selection(ctx: &CommandContext) -> bool {
    ctx.has_single_selection()
}
