use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::context::CommandContext;
use crate::descriptor::{CommandDescriptor, CommandMatch};
use crate::id::CommandId;
use crate::keybinding::{KeyBinding, current_platform};

use super::scoring::{
    build_command_haystack, category_relevance, context_boost, input_mode_for,
};
use super::usage::UsageTracker;

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
        super::nav::register_navigation(&mut descriptors);
        super::email::register_email(&mut descriptors);
        super::compose::register_compose(&mut descriptors);
        super::tasks::register_tasks(&mut descriptors);
        super::view::register_view(&mut descriptors);
        super::calendar::register_calendar(&mut descriptors);
        super::app::register_app(&mut descriptors);
        super::smart_folders::register_smart_folders(&mut descriptors);

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
    ) -> Result<crate::input::ParamDef, String> {
        let desc = self
            .get(command_id)
            .ok_or_else(|| format!("unknown command: {command_id:?}"))?;
        let schema = desc
            .input_schema
            .ok_or_else(|| format!("{command_id:?} is not a parameterized command"))?;
        let param = schema.param_at(param_index).ok_or_else(|| {
            format!(
                "{command_id:?}: param_index {param_index} out of bounds (schema has {} steps)",
                schema.len()
            )
        })?;
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
                    palette_label: d.resolved_palette_label(ctx),
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
                let score = raw_score
                    .saturating_add(boost)
                    .saturating_add(availability_bonus);
                let recency = self.usage.usage_count(d.id);

                results.push(CommandMatch {
                    id: d.id,
                    label,
                    palette_label: d.resolved_palette_label(ctx),
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
