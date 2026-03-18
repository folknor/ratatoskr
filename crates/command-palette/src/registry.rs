use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::context::CommandContext;
use super::descriptor::{CommandDescriptor, CommandMatch};
use super::id::CommandId;
use super::input::{InputMode, InputSchema};
use super::keybinding::{current_platform, KeyBinding, NamedKey};

pub struct CommandRegistry {
    descriptors: Vec<CommandDescriptor>,
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
        register_app(&mut descriptors);

        #[cfg(debug_assertions)]
        {
            let mut seen = std::collections::HashSet::new();
            for d in &descriptors {
                assert!(seen.insert(d.id), "duplicate CommandId: {:?}", d.id);
            }
        }

        Self { descriptors }
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
            .map(|d| CommandMatch {
                id: d.id,
                label: d.resolved_label(ctx),
                category: d.category,
                keybinding: d.keybinding.map(|kb| kb.display(platform)),
                available: (d.is_available)(ctx),
                input_mode: input_mode_for(d),
                score: 0,
                match_positions: vec![],
            })
            .collect();
        results.sort_by(|a, b| a.category.cmp(b.category).then(a.label.cmp(b.label)));
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
            let haystack_str = format!("{} > {}", d.category, label);
            let haystack = Utf32Str::new(&haystack_str, &mut buf);

            if let Some(score) = pattern.score(haystack, &mut matcher) {
                indices.clear();
                indices.resize(pattern.atoms.len() * 2, 0);
                pattern.indices(haystack, &mut matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();

                results.push(CommandMatch {
                    id: d.id,
                    label,
                    category: d.category,
                    keybinding: d.keybinding.map(|kb| kb.display(platform)),
                    available: (d.is_available)(ctx),
                    input_mode: input_mode_for(d),
                    score,
                    match_positions: indices.clone(),
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
    }
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
    out.push(desc(CommandId::EmailArchive, "Archive", "Email", Some(KeyBinding::key('e')), needs_selection));
    out.push(desc(
        CommandId::EmailTrash,
        "Move to Trash",
        "Email",
        Some(KeyBinding::key('#')),
        |ctx| ctx.has_selection() && ctx.thread_in_trash != Some(true),
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
    out.push(toggle(
        CommandId::EmailMarkRead,
        "Mark as Read",
        "Mark as Unread",
        "Email",
        None,
        needs_selection,
        |ctx| ctx.thread_is_read == Some(true),
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
    out.push(parameterized(
        CommandId::EmailMoveToFolder,
        "Move to Folder",
        "Email",
        Some(KeyBinding::key('v')),
        needs_selection,
        InputSchema::Single {
            param: super::input::ParamDef::ListPicker { label: "Folder" },
        },
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
    out.push(desc(CommandId::ComposeNew, "Compose New Email", "Compose", Some(KeyBinding::key('c')), always));
    out.push(desc(CommandId::ComposeReply, "Reply", "Compose", Some(KeyBinding::key('r')), needs_selection));
    out.push(desc(CommandId::ComposeReplyAll, "Reply All", "Compose", Some(KeyBinding::key('a')), needs_selection));
    out.push(desc(CommandId::ComposeForward, "Forward", "Compose", Some(KeyBinding::key('f')), needs_selection));
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

fn register_app(out: &mut Vec<CommandDescriptor>) {
    out.push(desc(CommandId::AppSearch, "Search", "App", Some(KeyBinding::key('/')), always));
    out.push(desc(CommandId::AppAskAi, "Ask AI", "App", Some(KeyBinding::key('i')), always));
    out.push(desc(CommandId::AppHelp, "Keyboard Shortcuts", "App", Some(KeyBinding::key('?')), always));
    out.push(desc(
        CommandId::AppSyncFolder,
        "Sync Current Folder",
        "App",
        Some(KeyBinding::named(NamedKey::F5)),
        |ctx| ctx.active_account_id.is_some(),
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
    fn empty_query_sorted_by_category_then_label() {
        let registry = CommandRegistry::new();
        let ctx = empty_context();
        let results = registry.query(&ctx, "");
        for window in results.windows(2) {
            let a = &window[0];
            let b = &window[1];
            assert!(
                (a.category, a.label) <= (b.category, b.label),
                "unsorted: {} > {} before {} > {}",
                a.category,
                a.label,
                b.category,
                b.label
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
}
