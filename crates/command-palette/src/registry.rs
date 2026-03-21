use std::collections::HashMap;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::context::{CommandContext, FocusedRegion, ViewType};
use super::descriptor::{CommandDescriptor, CommandMatch};
use super::id::CommandId;
use super::input::{InputMode, InputSchema};
use super::keybinding::{current_platform, KeyBinding, NamedKey};

/// Tracks command usage counts for recency/frequency ranking.
///
/// Persistence is deferred to Slice 6 — the app layer will be responsible
/// for saving and restoring this data.
pub struct UsageTracker {
    counts: HashMap<CommandId, u32>,
}

impl UsageTracker {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    pub fn record_usage(&mut self, id: CommandId) {
        *self.counts.entry(id).or_insert(0) += 1;
    }

    pub fn usage_count(&self, id: CommandId) -> u32 {
        self.counts.get(&id).copied().unwrap_or(0)
    }
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self::new()
    }
}

pub struct CommandRegistry {
    descriptors: Vec<CommandDescriptor>,
    pub usage: UsageTracker,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    pub fn new() -> Self {
        let mut descriptors = Vec::with_capacity(55);
        register_navigation(&mut descriptors);
        register_email(&mut descriptors);
        register_compose(&mut descriptors);
        register_tasks(&mut descriptors);
        register_view(&mut descriptors);
        register_calendar(&mut descriptors);
        register_app(&mut descriptors);
        register_smart_folders(&mut descriptors);

        #[cfg(debug_assertions)]
        {
            let mut seen = std::collections::HashSet::new();
            for d in &descriptors {
                assert!(seen.insert(d.id), "duplicate CommandId: {:?}", d.id);
            }
        }

        Self {
            descriptors,
            usage: UsageTracker::new(),
        }
    }

    pub fn get(&self, id: CommandId) -> Option<&CommandDescriptor> {
        self.descriptors.iter().find(|d| d.id == id)
    }

    /// Iterate over `(CommandId, KeyBinding)` for all commands with default
    /// bindings. Used by `BindingTable::new()` to populate defaults.
    pub fn default_bindings(&self) -> impl Iterator<Item = (CommandId, KeyBinding)> + '_ {
        self.descriptors
            .iter()
            .filter_map(|d| d.keybinding.map(|kb| (d.id, kb)))
    }

    /// Validate that `command_id` is a parameterized command and that
    /// `param_index` is within its schema bounds. Returns the `ParamDef`
    /// for the step on success.
    ///
    /// Also validates that `prior_selections` length matches `param_index`
    /// (one prior value per completed step).
    pub fn validate_param_request(
        &self,
        command_id: CommandId,
        param_index: usize,
        prior_selections: &[String],
    ) -> Result<super::input::ParamDef, String> {
        let desc = self
            .get(command_id)
            .ok_or_else(|| format!("unknown command: {command_id:?}"))?;
        let schema = desc
            .input_schema
            .ok_or_else(|| format!("{command_id:?} is not a parameterized command"))?;
        let param = schema
            .param_at(param_index)
            .ok_or_else(|| format!("{command_id:?}: param_index {param_index} out of bounds (schema has {} steps)", schema.len()))?;
        if prior_selections.len() != param_index {
            return Err(format!(
                "{command_id:?}: expected {} prior selections for step {param_index}, got {}",
                param_index,
                prior_selections.len()
            ));
        }
        Ok(param)
    }

    pub fn query(&self, ctx: &CommandContext, query: &str) -> Vec<CommandMatch> {
        if query.is_empty() {
            return self.query_empty(ctx);
        }
        self.query_fuzzy(ctx, query)
    }

    fn query_empty(&self, ctx: &CommandContext) -> Vec<CommandMatch> {
        let platform = current_platform();
        let mut results: Vec<CommandMatch> = self
            .descriptors
            .iter()
            .map(|d| {
                let recency = self.usage.usage_count(d.id);
                CommandMatch {
                    id: d.id,
                    label: d.resolved_label(ctx),
                    category: d.category,
                    keybinding: d.keybinding.map(|kb| kb.display(platform)),
                    available: (d.is_available)(ctx),
                    input_mode: input_mode_for(d),
                    score: 0,
                    match_positions: vec![],
                    recency_score: recency,
                }
            })
            .collect();
        results.sort_by(|a, b| {
            b.available
                .cmp(&a.available)
                .then_with(|| b.recency_score.cmp(&a.recency_score))
                .then_with(|| {
                    let a_rel = category_relevance(a.category, ctx);
                    let b_rel = category_relevance(b.category, ctx);
                    b_rel.cmp(&a_rel)
                })
                .then_with(|| a.category.cmp(b.category))
                .then_with(|| a.label.cmp(b.label))
        });
        results
    }

    fn query_fuzzy(&self, ctx: &CommandContext, query: &str) -> Vec<CommandMatch> {
        let platform = current_platform();
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut indices = Vec::new();
        let mut results = Vec::new();

        for d in &self.descriptors {
            let label = d.resolved_label(ctx);
            let haystack_str = build_command_haystack(d.category, label, d.keywords);
            let haystack = Utf32Str::new(&haystack_str, &mut buf);

            if let Some(raw_score) = pattern.score(haystack, &mut matcher) {
                indices.clear();
                indices.resize(pattern.atoms.len() * 2, 0);
                pattern.indices(haystack, &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();

                let available = (d.is_available)(ctx);
                let boost = context_boost(d, ctx);
                let availability_bonus = if available { 1000 } else { 0 };
                let score = raw_score.saturating_add(boost).saturating_add(availability_bonus);
                let recency = self.usage.usage_count(d.id);

                results.push(CommandMatch {
                    id: d.id,
                    label,
                    category: d.category,
                    keybinding: d.keybinding.map(|kb| kb.display(platform)),
                    available,
                    input_mode: input_mode_for(d),
                    score,
                    match_positions: indices.clone(),
                    recency_score: recency,
                });
            }
        }

        results.sort_by_key(|r| std::cmp::Reverse(r.score));
        results
    }
}

fn input_mode_for(d: &CommandDescriptor) -> InputMode {
    match d.input_schema {
        Some(schema) => InputMode::Parameterized { schema },
        None => InputMode::Direct,
    }
}

fn build_command_haystack(category: &str, label: &str, keywords: &[&str]) -> String {
    let mut haystack = format!("{category} > {label}");
    for kw in keywords {
        haystack.push(' ');
        haystack.push_str(kw);
    }
    haystack
}

/// Returns a modest score boost when the command's category aligns with
/// the current view context or focused region.
fn context_boost(descriptor: &CommandDescriptor, ctx: &CommandContext) -> u32 {
    let mut boost: u32 = 0;
    boost += category_relevance(descriptor.category, ctx) * 5;
    boost += focused_region_boost(descriptor.category, ctx);
    boost
}

/// Returns 0-4 relevance points for how well a category matches the view.
fn category_relevance(category: &str, ctx: &CommandContext) -> u32 {
    match category {
        "Email" => email_view_relevance(ctx),
        "Compose" => compose_view_relevance(ctx),
        "Navigation" => 2, // always somewhat relevant
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
        | ViewType::Label
        | ViewType::SmartFolder
        | ViewType::Category
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

fn always(_ctx: &CommandContext) -> bool {
    true
}

fn needs_selection(ctx: &CommandContext) -> bool {
    ctx.has_selection()
}

fn needs_single_selection(ctx: &CommandContext) -> bool {
    ctx.has_single_selection()
}

fn desc(
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
    }
}

fn desc_kw(
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
    }
}

fn toggle(
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
    }
}

fn with_keywords(mut d: CommandDescriptor, keywords: &'static [&'static str]) -> CommandDescriptor {
    d.keywords = keywords;
    d
}

fn parameterized(
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
    }
}

fn register_navigation(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(CommandId::NavNext, "Next Thread", "Navigation", Some(KeyBinding::key('j')), always));
    out.push(desc(CommandId::NavPrev, "Previous Thread", "Navigation", Some(KeyBinding::key('k')), always));
    out.push(desc(CommandId::NavOpen, "Open Thread", "Navigation", Some(KeyBinding::key('o')), needs_selection));
    out.push(desc(
        CommandId::NavMsgNext,
        "Next Message",
        "Navigation",
        Some(KeyBinding::named(NamedKey::ArrowDown)),
        |ctx| ctx.active_message_id.is_some(),
    ));
    out.push(desc(
        CommandId::NavMsgPrev,
        "Previous Message",
        "Navigation",
        Some(KeyBinding::named(NamedKey::ArrowUp)),
        |ctx| ctx.active_message_id.is_some(),
    ));
    out.push(desc(CommandId::NavGoInbox, "Go to Inbox", "Navigation", Some(KeyBinding::seq('g', 'i')), always));
    out.push(desc(CommandId::NavGoStarred, "Go to Starred", "Navigation", Some(KeyBinding::seq('g', 's')), always));
    out.push(desc(CommandId::NavGoSent, "Go to Sent", "Navigation", Some(KeyBinding::seq('g', 't')), always));
    out.push(desc(CommandId::NavGoDrafts, "Go to Drafts", "Navigation", Some(KeyBinding::seq('g', 'd')), always));
    out.push(desc(CommandId::NavGoSnoozed, "Go to Snoozed", "Navigation", None, always));
    out.push(desc(CommandId::NavGoTrash, "Go to Trash", "Navigation", None, always));
    out.push(desc(CommandId::NavGoAllMail, "Go to All Mail", "Navigation", None, always));
    out.push(parameterized(
        CommandId::NavigateToLabel,
        "Go to Label",
        "Navigation",
        Some(KeyBinding::seq('g', 'l')),
        always,
        InputSchema::Single {
            param: super::input::ParamDef::ListPicker { label: "Label" },
        },
    ));
    register_navigation_categories(out);
}

fn register_navigation_categories(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(CommandId::NavGoPrimary, "Go to Primary", "Navigation", Some(KeyBinding::seq('g', 'p')), always));
    out.push(desc(CommandId::NavGoUpdates, "Go to Updates", "Navigation", Some(KeyBinding::seq('g', 'u')), always));
    out.push(desc(
        CommandId::NavGoPromotions,
        "Go to Promotions",
        "Navigation",
        Some(KeyBinding::seq('g', 'o')),
        always,
    ));
    out.push(desc(CommandId::NavGoSocial, "Go to Social", "Navigation", Some(KeyBinding::seq('g', 'c')), always));
    out.push(desc(
        CommandId::NavGoNewsletters,
        "Go to Newsletters",
        "Navigation",
        Some(KeyBinding::seq('g', 'n')),
        always,
    ));
    out.push(desc(CommandId::NavGoTasks, "Go to Tasks", "Navigation", Some(KeyBinding::seq('g', 'k')), always));
    out.push(desc(
        CommandId::NavGoAttachments,
        "Go to Attachments",
        "Navigation",
        Some(KeyBinding::seq('g', 'a')),
        always,
    ));
    out.push(desc(
        CommandId::NavEscape,
        "Close / Go Back",
        "Navigation",
        Some(KeyBinding::named(NamedKey::Escape)),
        |ctx| ctx.has_selection() || ctx.composer_is_open,
    ));
}

fn register_email(out: &mut Vec<CommandDescriptor>) {
    out.push(desc_kw(
        CommandId::EmailArchive, "Archive", "Email",
        Some(KeyBinding::key('e')), needs_selection,
        &["done", "file"],
    ));
    out.push(desc_kw(
        CommandId::EmailTrash,
        "Move to Trash",
        "Email",
        Some(KeyBinding::key('#')),
        |ctx| ctx.has_selection() && ctx.thread_in_trash != Some(true),
        &["delete", "remove"],
    ));
    out.push(desc(
        CommandId::EmailPermanentDelete,
        "Permanently Delete",
        "Email",
        None,
        |ctx| ctx.has_selection() && ctx.thread_in_trash == Some(true),
    ));
    out.push(toggle(
        CommandId::EmailSpam,
        "Report Spam",
        "Not Spam",
        "Email",
        Some(KeyBinding::key('!')),
        needs_selection,
        |ctx| ctx.thread_in_spam == Some(true),
    ));
    register_email_toggles(out);
    register_email_other(out);
}

fn register_email_toggles(out: &mut Vec<CommandDescriptor>) {
    out.push(with_keywords(
        toggle(
            CommandId::EmailMarkRead,
            "Mark as Read",
            "Mark as Unread",
            "Email",
            None,
            needs_selection,
            |ctx| ctx.thread_is_read == Some(true),
        ),
        &["seen"],
    ));
    out.push(toggle(
        CommandId::EmailStar,
        "Star",
        "Unstar",
        "Email",
        Some(KeyBinding::key('s')),
        needs_selection,
        |ctx| ctx.thread_is_starred == Some(true),
    ));
    out.push(toggle(
        CommandId::EmailPin,
        "Pin",
        "Unpin",
        "Email",
        Some(KeyBinding::key('p')),
        needs_selection,
        |ctx| ctx.thread_is_pinned == Some(true),
    ));
    out.push(toggle(
        CommandId::EmailMute,
        "Mute",
        "Unmute",
        "Email",
        Some(KeyBinding::key('m')),
        needs_selection,
        |ctx| ctx.thread_is_muted == Some(true),
    ));
}

fn register_email_other(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(
        CommandId::EmailUnsubscribe,
        "Unsubscribe",
        "Email",
        Some(KeyBinding::key('u')),
        needs_single_selection,
    ));
    out.push(with_keywords(
        parameterized(
            CommandId::EmailMoveToFolder,
            "Move to Folder",
            "Email",
            Some(KeyBinding::key('v')),
            needs_selection,
            InputSchema::Single {
                param: super::input::ParamDef::ListPicker { label: "Folder" },
            },
        ),
        &["file", "organize"],
    ));
    out.push(parameterized(
        CommandId::EmailAddLabel,
        "Add Label",
        "Email",
        None,
        needs_selection,
        InputSchema::Single {
            param: super::input::ParamDef::ListPicker { label: "Label" },
        },
    ));
    out.push(parameterized(
        CommandId::EmailRemoveLabel,
        "Remove Label",
        "Email",
        None,
        needs_selection,
        InputSchema::Single {
            param: super::input::ParamDef::ListPicker { label: "Label" },
        },
    ));
    out.push(parameterized(
        CommandId::EmailSnooze,
        "Snooze",
        "Email",
        None,
        needs_selection,
        InputSchema::Single {
            param: super::input::ParamDef::DateTime { label: "Snooze until" },
        },
    ));
    out.push(desc(CommandId::EmailSelectAll, "Select All", "Email", Some(KeyBinding::cmd_or_ctrl('a')), always));
    out.push(desc(
        CommandId::EmailSelectFromHere,
        "Select All From Here",
        "Email",
        Some(KeyBinding::cmd_or_ctrl_shift('a')),
        needs_selection,
    ));
}

fn register_compose(out: &mut Vec<CommandDescriptor>) {
    out.push(desc_kw(
        CommandId::ComposeNew, "Compose New Email", "Compose",
        Some(KeyBinding::key('c')), always,
        &["write", "new", "create"],
    ));
    out.push(desc_kw(
        CommandId::ComposeReply, "Reply", "Compose",
        Some(KeyBinding::key('r')), needs_selection,
        &["respond"],
    ));
    out.push(desc(CommandId::ComposeReplyAll, "Reply All", "Compose", Some(KeyBinding::key('a')), needs_selection));
    out.push(desc_kw(
        CommandId::ComposeForward, "Forward", "Compose",
        Some(KeyBinding::key('f')), needs_selection,
        &["send", "share"],
    ));
}

fn register_tasks(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(CommandId::TaskCreate, "Create Task", "Tasks", None, always));
    out.push(desc(
        CommandId::TaskCreateFromEmail,
        "Create Task from Email",
        "Tasks",
        Some(KeyBinding::key('t')),
        needs_selection,
    ));
    out.push(desc(CommandId::TaskTogglePanel, "Toggle Task Panel", "Tasks", None, always));
    // No default binding — NavGoTasks already owns "g then k".
    out.push(desc(CommandId::TaskViewAll, "View All Tasks", "Tasks", None, always));
}

fn register_view(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(
        CommandId::ViewToggleSidebar,
        "Toggle Sidebar",
        "View",
        Some(KeyBinding::cmd_or_ctrl_shift('e')),
        always,
    ));
    out.push(desc(CommandId::ViewSetThemeLight, "Light Theme", "View", None, always));
    out.push(desc(CommandId::ViewSetThemeDark, "Dark Theme", "View", None, always));
    out.push(desc(CommandId::ViewSetThemeSystem, "System Theme", "View", None, always));
    out.push(desc(CommandId::ViewToggleTaskPanel, "Toggle Task Panel", "View", None, always));
    register_view_reading_pane(out);
}

fn register_view_reading_pane(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(
        CommandId::ViewReadingPaneRight,
        "Reading Pane Right",
        "View",
        None,
        always,
    ));
    out.push(desc(
        CommandId::ViewReadingPaneBottom,
        "Reading Pane Bottom",
        "View",
        None,
        always,
    ));
    out.push(desc(
        CommandId::ViewReadingPaneHidden,
        "Reading Pane Hidden",
        "View",
        None,
        always,
    ));
}

fn register_calendar(out: &mut Vec<CommandDescriptor>) {
    out.push(desc_kw(
        CommandId::CalendarToggle,
        "Toggle Calendar",
        "Calendar",
        Some(KeyBinding::cmd_or_ctrl('2')),
        always,
        &["switch mode", "mail", "calendar"],
    ));
    out.push(desc(CommandId::CalendarViewDay, "Day View", "Calendar", None, always));
    out.push(desc(CommandId::CalendarViewWorkWeek, "Work Week View", "Calendar", None, always));
    out.push(desc(CommandId::CalendarViewWeek, "Week View", "Calendar", None, always));
    out.push(desc(CommandId::CalendarViewMonth, "Month View", "Calendar", None, always));
    out.push(desc_kw(
        CommandId::CalendarToday,
        "Go to Today",
        "Calendar",
        None,
        always,
        &["today", "now", "current date"],
    ));
    out.push(desc_kw(
        CommandId::CalendarCreateEvent,
        "Create Event",
        "Calendar",
        None,
        always,
        &["new event", "add event"],
    ));
}

fn register_smart_folders(out: &mut Vec<CommandDescriptor>) {
    out.push(with_keywords(
        parameterized(
            CommandId::SmartFolderSave,
            "Save as Smart Folder",
            "Search",
            None,
            |ctx| ctx.search_query.as_ref().is_some_and(|q| !q.is_empty()),
            InputSchema::Single {
                param: super::input::ParamDef::Text {
                    label: "Name",
                    placeholder: "Smart folder name...",
                },
            },
        ),
        &["smart folder", "save search", "pin"],
    ));
}

fn register_app(out: &mut Vec<CommandDescriptor>) {
    out.push(desc_kw(CommandId::AppSearch, "Search", "App", Some(KeyBinding::key('/')), always, &["find", "ctrl+f"]));
    out.push(desc(CommandId::AppAskAi, "Ask AI", "App", Some(KeyBinding::key('i')), always));
    out.push(desc(CommandId::AppHelp, "Keyboard Shortcuts", "App", Some(KeyBinding::key('?')), always));
    out.push(desc(
        CommandId::AppSyncFolder,
        "Sync Current Folder",
        "App",
        Some(KeyBinding::named(NamedKey::F5)),
        |ctx| ctx.active_account_id.is_some(),
    ));
    out.push(desc_kw(
        CommandId::AppOpenPalette,
        "Command Palette",
        "App",
        Some(KeyBinding::cmd_or_ctrl('k')),
        always,
        &["palette", "commands"],
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ViewType;
    use crate::input::InputMode;

    fn empty_context() -> CommandContext {
        CommandContext {
            selected_thread_ids: vec![],
            active_message_id: None,
            current_view: ViewType::Inbox,
            current_label_id: None,
            active_account_id: None,
            provider_kind: None,
            thread_is_read: None,
            thread_is_starred: None,
            thread_is_muted: None,
            thread_is_pinned: None,
            thread_is_draft: None,
            thread_in_trash: None,
            thread_in_spam: None,
            is_online: true,
            composer_is_open: false,
            focused_region: None,
            search_query: None,
        }
    }

    fn context_with_selection() -> CommandContext {
        let mut ctx = empty_context();
        ctx.selected_thread_ids = vec!["thread-1".to_string()];
        ctx.active_account_id = Some("acc-1".to_string());
        ctx
    }

    #[test]
    fn registry_covers_all_command_ids() {
        let registry = CommandRegistry::new();
        for id in CommandId::all() {
            assert!(
                registry.get(*id).is_some(),
                "CommandId::{id:?} not registered"
            );
        }
    }

    #[test]
    fn empty_query_returns_all_commands() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "");
        assert_eq!(results.len(), CommandId::all().len());
    }

    #[test]
    fn empty_query_available_first_then_alphabetical() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "");

        // Available commands come before unavailable ones
        let first_unavailable = results.iter().position(|r| !r.available);
        let last_available = results.iter().rposition(|r| r.available);
        if let (Some(first_un), Some(last_av)) = (first_unavailable, last_available) {
            assert!(
                last_av < first_un,
                "available commands should precede unavailable ones"
            );
        }

        // Within the available group, alphabetical by (category, label)
        let available: Vec<_> = results.iter().filter(|r| r.available).collect();
        for window in available.windows(2) {
            let a = window[0];
            let b = window[1];
            // Context-relevant categories may come before less relevant ones,
            // but within same relevance tier, alphabetical order holds.
            assert!(
                (a.category, a.label) <= (b.category, b.label)
                    || category_relevance(a.category, &ctx)
                        >= category_relevance(b.category, &ctx),
                "available group misordered: {} > {} before {} > {}",
                a.category, a.label, b.category, b.label
            );
        }
    }

    #[test]
    fn no_selection_marks_email_unavailable() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(archive.is_some());
        assert!(!archive.map_or(true, |a| a.available));
    }

    #[test]
    fn selection_marks_email_available() {
        let registry = CommandRegistry::new();
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(archive.map_or(false, |a| a.available));
    }

    #[test]
    fn trash_vs_permanent_delete_exclusivity() {
        let registry = CommandRegistry::new();

        // Not in trash: Trash available, PermanentDelete not
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "");
        let trash = results.iter().find(|r| r.id == CommandId::EmailTrash);
        let perm = results.iter().find(|r| r.id == CommandId::EmailPermanentDelete);
        assert!(trash.map_or(false, |t| t.available));
        assert!(!perm.map_or(true, |p| p.available));

        // In trash: Trash not available, PermanentDelete available
        let mut ctx2 = context_with_selection();
        ctx2.thread_in_trash = Some(true);
        let results2 = registry.query(&ctx2, "");
        let trash2 = results2.iter().find(|r| r.id == CommandId::EmailTrash);
        let perm2 = results2
            .iter()
            .find(|r| r.id == CommandId::EmailPermanentDelete);
        assert!(!trash2.map_or(true, |t| t.available));
        assert!(perm2.map_or(false, |p| p.available));
    }

    #[test]
    fn fuzzy_search_finds_archive() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "arch");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(archive.is_some());
        assert!(archive.map_or(false, |a| a.score > 0));
    }

    #[test]
    fn fuzzy_first_letter_matching() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "ea");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(
            archive.is_some(),
            "\"ea\" should match \"Email > Archive\" via first-letter-of-each-word"
        );
    }

    #[test]
    fn fuzzy_results_sorted_by_score_desc() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "go");
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "results not sorted by score desc"
            );
        }
    }

    #[test]
    fn toggle_label_resolves_based_on_state() {
        let registry = CommandRegistry::new();

        // Not starred → label is "Star"
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "");
        let star = results.iter().find(|r| r.id == CommandId::EmailStar);
        assert_eq!(star.map(|s| s.label), Some("Star"));

        // Starred → label is "Unstar"
        let mut ctx2 = context_with_selection();
        ctx2.thread_is_starred = Some(true);
        let results2 = registry.query(&ctx2, "");
        let star2 = results2.iter().find(|r| r.id == CommandId::EmailStar);
        assert_eq!(star2.map(|s| s.label), Some("Unstar"));
    }

    #[test]
    fn parameterized_command_has_input_mode() {
        let registry = CommandRegistry::new();
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "");
        let move_to = results
            .iter()
            .find(|r| r.id == CommandId::EmailMoveToFolder);
        assert!(move_to.is_some());
        assert!(
            matches!(
                move_to.map(|m| m.input_mode),
                Some(InputMode::Parameterized { .. })
            ),
            "EmailMoveToFolder should be Parameterized"
        );
    }

    #[test]
    fn direct_command_has_direct_mode() {
        let registry = CommandRegistry::new();
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(archive.is_some());
        assert!(
            matches!(
                archive.map(|m| m.input_mode),
                Some(InputMode::Direct)
            ),
            "EmailArchive should be Direct"
        );
    }

    #[test]
    fn validate_param_request_accepts_valid_list_picker() {
        let registry = CommandRegistry::new();
        let result = registry.validate_param_request(CommandId::EmailMoveToFolder, 0, &[]);
        assert!(result.is_ok());
        assert!(result.map_or(false, |p| p.is_list_picker()));
    }

    #[test]
    fn validate_param_request_rejects_non_parameterized() {
        let registry = CommandRegistry::new();
        let result = registry.validate_param_request(CommandId::EmailArchive, 0, &[]);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("not a parameterized command"),
            "should reject non-parameterized command"
        );
    }

    #[test]
    fn validate_param_request_rejects_out_of_bounds() {
        let registry = CommandRegistry::new();
        let result = registry.validate_param_request(CommandId::EmailMoveToFolder, 1, &["x".into()]);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("out of bounds"),
            "should reject out-of-bounds param_index"
        );
    }

    #[test]
    fn validate_param_request_rejects_wrong_prior_selections_count() {
        let registry = CommandRegistry::new();
        // Step 0 should have 0 prior selections, not 1
        let result = registry.validate_param_request(
            CommandId::EmailMoveToFolder,
            0,
            &["unexpected".into()],
        );
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("prior selections"),
            "should reject mismatched prior_selections count"
        );
    }

    #[test]
    fn sync_folder_needs_account() {
        let registry = CommandRegistry::new();

        let ctx = empty_context();
        let results = registry.query(&ctx, "");
        let sync = results.iter().find(|r| r.id == CommandId::AppSyncFolder);
        assert!(!sync.map_or(true, |s| s.available));

        let mut ctx2 = empty_context();
        ctx2.active_account_id = Some("acc-1".to_string());
        let results2 = registry.query(&ctx2, "");
        let sync2 = results2.iter().find(|r| r.id == CommandId::AppSyncFolder);
        assert!(sync2.map_or(false, |s| s.available));
    }

    // --- Slice 4: Ranking signals ---

    #[test]
    fn keyword_search_finds_trash_via_delete() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "delete");
        let trash = results.iter().find(|r| r.id == CommandId::EmailTrash);
        assert!(
            trash.is_some(),
            "\"delete\" should match Move to Trash via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_archive_via_done() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "done");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(
            archive.is_some(),
            "\"done\" should match Archive via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_compose_via_write() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "write");
        let compose = results.iter().find(|r| r.id == CommandId::ComposeNew);
        assert!(
            compose.is_some(),
            "\"write\" should match Compose New Email via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_mark_read_via_seen() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "seen");
        let mark_read = results.iter().find(|r| r.id == CommandId::EmailMarkRead);
        assert!(
            mark_read.is_some(),
            "\"seen\" should match Mark as Read via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_move_to_folder_via_organize() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "organize");
        let m = results
            .iter()
            .find(|r| r.id == CommandId::EmailMoveToFolder);
        assert!(
            m.is_some(),
            "\"organize\" should match Move to Folder via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_reply_via_respond() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "respond");
        let reply = results.iter().find(|r| r.id == CommandId::ComposeReply);
        assert!(
            reply.is_some(),
            "\"respond\" should match Reply via keyword"
        );
    }

    #[test]
    fn keyword_search_finds_forward_via_share() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "share");
        let fwd = results.iter().find(|r| r.id == CommandId::ComposeForward);
        assert!(
            fwd.is_some(),
            "\"share\" should match Forward via keyword"
        );
    }

    #[test]
    fn available_commands_rank_above_unavailable_in_fuzzy() {
        let registry = CommandRegistry::new();
        // With selection: Archive is available
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "arch");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(archive.map_or(false, |a| a.available));
        assert!(archive.map_or(false, |a| a.score >= 1000));

        // Without selection: Archive is unavailable
        let ctx2 = empty_context();
        let results2 = registry.query(&ctx2, "arch");
        let archive2 = results2.iter().find(|r| r.id == CommandId::EmailArchive);
        assert!(!archive2.map_or(true, |a| a.available));
        assert!(archive2.map_or(true, |a| a.score < 1000));
    }

    #[test]
    fn context_boost_favors_email_in_inbox() {
        let registry = CommandRegistry::new();
        let ctx = context_with_selection();
        let results = registry.query(&ctx, "arch");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        // Email category gets a context boost when viewing Inbox
        let score = archive.map_or(0, |a| a.score);
        // Score should include availability bonus (1000) + context boost
        assert!(
            score > 1000,
            "archive score {score} should exceed 1000 (availability bonus + context boost)"
        );
    }

    #[test]
    fn composer_focus_boosts_compose_commands() {
        let registry = CommandRegistry::new();
        let mut ctx = context_with_selection();
        ctx.focused_region = Some(FocusedRegion::Composer);
        ctx.composer_is_open = true;
        let results = registry.query(&ctx, "reply");
        let reply = results.iter().find(|r| r.id == CommandId::ComposeReply);
        let reply_score = reply.map_or(0, |r| r.score);

        // Same query without composer focus
        let ctx2 = context_with_selection();
        let results2 = registry.query(&ctx2, "reply");
        let reply2 = results2.iter().find(|r| r.id == CommandId::ComposeReply);
        let reply_score2 = reply2.map_or(0, |r| r.score);

        assert!(
            reply_score > reply_score2,
            "compose commands should score higher with composer focused: {reply_score} vs {reply_score2}"
        );
    }

    #[test]
    fn empty_query_groups_available_before_unavailable() {
        let registry = CommandRegistry::new();
        // No selection: email actions are unavailable, nav/app/view are available
        let ctx = empty_context();
        let results = registry.query(&ctx, "");

        let mut seen_unavailable = false;
        for r in &results {
            if !r.available {
                seen_unavailable = true;
            } else if seen_unavailable {
                panic!(
                    "available command {} > {} after unavailable ones",
                    r.category, r.label
                );
            }
        }
    }

    #[test]
    fn tasks_view_boosts_tasks_category() {
        let mut ctx = empty_context();
        ctx.current_view = ViewType::Tasks;
        let relevance = category_relevance("Tasks", &ctx);
        assert_eq!(relevance, 4, "Tasks category should have max relevance on Tasks view");

        let ctx2 = empty_context(); // Inbox
        let relevance2 = category_relevance("Tasks", &ctx2);
        assert!(
            relevance > relevance2,
            "Tasks relevance should be higher on Tasks view ({relevance}) than Inbox ({relevance2})"
        );
    }

    #[test]
    fn recency_score_populated_from_usage_tracker() {
        let mut registry = CommandRegistry::new();
        registry.usage.record_usage(CommandId::EmailArchive);
        registry.usage.record_usage(CommandId::EmailArchive);
        registry.usage.record_usage(CommandId::ComposeNew);

        let ctx = empty_context();
        let results = registry.query(&ctx, "");

        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert_eq!(archive.map_or(0, |a| a.recency_score), 2);

        let compose = results.iter().find(|r| r.id == CommandId::ComposeNew);
        assert_eq!(compose.map_or(0, |c| c.recency_score), 1);

        let star = results.iter().find(|r| r.id == CommandId::EmailStar);
        assert_eq!(star.map_or(99, |s| s.recency_score), 0);
    }

    #[test]
    fn usage_tracker_basics() {
        let mut tracker = UsageTracker::new();
        assert_eq!(tracker.usage_count(CommandId::NavNext), 0);

        tracker.record_usage(CommandId::NavNext);
        assert_eq!(tracker.usage_count(CommandId::NavNext), 1);

        tracker.record_usage(CommandId::NavNext);
        tracker.record_usage(CommandId::NavNext);
        assert_eq!(tracker.usage_count(CommandId::NavNext), 3);

        // Other commands unaffected
        assert_eq!(tracker.usage_count(CommandId::NavPrev), 0);
    }

    #[test]
    fn recency_score_in_fuzzy_results() {
        let mut registry = CommandRegistry::new();
        registry.usage.record_usage(CommandId::EmailArchive);
        registry.usage.record_usage(CommandId::EmailArchive);
        registry.usage.record_usage(CommandId::EmailArchive);

        let ctx = empty_context();
        let results = registry.query(&ctx, "arch");
        let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
        assert_eq!(
            archive.map_or(0, |a| a.recency_score),
            3,
            "fuzzy results should carry recency_score"
        );
    }

    #[test]
    fn descriptor_keywords_field() {
        let registry = CommandRegistry::new();
        let archive = registry.get(CommandId::EmailArchive);
        assert!(
            archive.map_or(false, |d| d.keywords.contains(&"done")),
            "Archive should have 'done' keyword"
        );
        let trash = registry.get(CommandId::EmailTrash);
        assert!(
            trash.map_or(false, |d| d.keywords.contains(&"delete")),
            "Trash should have 'delete' keyword"
        );
        // Commands with no keywords should have empty slice
        let nav_next = registry.get(CommandId::NavNext);
        assert!(
            nav_next.map_or(false, |d| d.keywords.is_empty()),
            "NavNext should have no keywords"
        );
    }

    #[test]
    fn settings_view_reduces_email_relevance() {
        let mut ctx = empty_context();
        ctx.current_view = ViewType::Settings;
        let email_rel = category_relevance("Email", &ctx);
        assert!(
            email_rel < 4,
            "Email relevance should be low on Settings view"
        );

        ctx.current_view = ViewType::Inbox;
        let email_rel_inbox = category_relevance("Email", &ctx);
        assert_eq!(email_rel_inbox, 4);
    }
}
