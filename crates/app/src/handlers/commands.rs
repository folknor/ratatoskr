use iced::Task;

use std::sync::Arc;

use crate::command_dispatch;
use crate::db::Db;
use crate::{APP_DATA_DIR, App, Message};
use cmdk::{CommandArgs, CommandId, KeyBinding, OptionItem};
use rtsk::actions::{ActionOutcome, FolderId, MailOperation, TagId};
use rtsk::scope::ViewScope;

impl App {
    /// Save keybinding overrides to disk. Call this after any mutation
    /// to `self.binding_table` overrides (`set_override`, `unbind`,
    /// `remove_override`, `reset_all`).
    pub(crate) fn save_keybinding_overrides(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("keybindings.json");
        if let Err(e) = self.binding_table.save_overrides(&path) {
            eprintln!("warning: failed to save keybinding overrides: {e}");
        }
    }

    /// Set a keybinding override and persist to disk.
    /// Returns `Err(conflicting_id)` if the binding conflicts.
    pub(crate) fn set_keybinding(
        &mut self,
        id: CommandId,
        binding: KeyBinding,
    ) -> Result<(), CommandId> {
        self.binding_table.set_override(id, binding)?;
        self.save_keybinding_overrides();
        Ok(())
    }

    /// Unbind a command (explicit, no fallback to default) and persist.
    pub(crate) fn unbind_keybinding(&mut self, id: CommandId) {
        self.binding_table.unbind(id);
        self.save_keybinding_overrides();
    }

    /// Remove a keybinding override (revert to default) and persist.
    pub(crate) fn remove_keybinding_override(&mut self, id: CommandId) {
        self.binding_table.remove_override(id);
        self.save_keybinding_overrides();
    }

    /// Reset all keybinding overrides to defaults and persist.
    pub(crate) fn reset_all_keybindings(&mut self) {
        self.binding_table.reset_all();
        self.save_keybinding_overrides();
    }

    pub(crate) fn handle_execute_command(&mut self, id: CommandId) -> Task<Message> {
        log::debug!("Executing command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_command(id, self) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    pub(crate) fn handle_execute_parameterized(
        &mut self,
        id: CommandId,
        args: CommandArgs,
    ) -> Task<Message> {
        log::debug!("Executing parameterized command: {id:?}");
        self.registry.usage.record_usage(id);
        self.save_usage_counts();
        match command_dispatch::dispatch_parameterized(id, args) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    /// Save usage counts to disk.
    fn save_usage_counts(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let path = data_dir.join("command_usage.json");
        let map = self.registry.usage.to_map();
        match serde_json::to_string(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    log::warn!("Failed to save usage counts: {e}");
                }
            }
            Err(e) => log::warn!("Failed to serialize usage counts: {e}"),
        }
    }

    /// Get a clone of the action context, or `None` if the action service
    /// is not initialized (degraded mode - stores failed at boot).
    pub(crate) fn action_ctx(&self) -> Option<rtsk::actions::ActionContext> {
        self.action_ctx.as_ref().cloned()
    }

    pub(crate) fn handle_email_action(
        &mut self,
        intent: crate::action_resolve::MailActionIntent,
    ) -> Task<Message> {
        // Public folder items are not real threads - actions don't apply.
        if matches!(self.sidebar.selected_scope, ViewScope::PublicFolder { .. }) {
            return Task::none();
        }

        // Collect selected thread info.
        let selected_threads: Vec<(String, String)> = self
            .thread_list
            .selected_indices()
            .iter()
            .filter_map(|&i| self.thread_list.threads.get(i))
            .map(|t| (t.account_id.clone(), t.id.clone()))
            .collect();

        if selected_threads.is_empty() {
            return Task::none();
        }

        // ── Resolve → Plan → Dispatch ───────────────────────────
        use crate::action_resolve::{self as ar, OptimisticMutation, UiContext};
        let ui_ctx = UiContext {
            selection: self.sidebar.selection.clone(),
        };
        let outcome = ar::resolve_intent(intent, &ui_ctx);

        let Some(plan) =
            ar::build_execution_plan(outcome, &selected_threads, &mut self.thread_list)
        else {
            // NoOp (Unsubscribe)
            self.status_bar
                .show_confirmation("Unsubscribed".to_string());
            return Task::none();
        };

        // Star optimistic UI → sync reading pane
        if plan
            .optimistic
            .iter()
            .any(|m| matches!(m, OptimisticMutation::SetStarred { .. }))
        {
            for m in &plan.optimistic {
                if let OptimisticMutation::SetStarred {
                    account_id,
                    thread_id,
                    previous,
                } = m
                {
                    self.reading_pane
                        .update_star(account_id, thread_id, !previous);
                }
            }
        }

        self.dispatch_plan(plan)
    }

    /// Dispatch an execution plan through the batch executor.
    pub(crate) fn dispatch_plan(
        &mut self,
        plan: crate::action_resolve::ActionExecutionPlan,
    ) -> Task<Message> {
        if plan.operations.is_empty() {
            return Task::none();
        }

        let Some(ctx) = self.action_ctx() else {
            // I4: rollback optimistic mutations before returning
            self.rollback_optimistic(&plan.optimistic);
            self.status_bar.show_confirmation(
                "\u{26A0} Action unavailable \u{2014} service not initialized".to_string(),
            );
            return Task::none();
        };

        let operations = plan.operations.clone();

        Task::perform(
            async move { rtsk::actions::batch_execute(&ctx, operations).await },
            move |outcomes| Message::ActionCompleted { plan, outcomes },
        )
    }

    /// Rollback optimistic mutations when dispatch cannot proceed (I4).
    fn rollback_optimistic(&mut self, mutations: &[crate::action_resolve::OptimisticMutation]) {
        use crate::action_resolve::OptimisticMutation;
        for m in mutations {
            match m {
                OptimisticMutation::SetStarred {
                    account_id,
                    thread_id,
                    previous,
                } => {
                    if let Some(t) = self
                        .thread_list
                        .threads
                        .iter_mut()
                        .find(|t| t.account_id == *account_id && t.id == *thread_id)
                    {
                        t.is_starred = *previous;
                    }
                    self.reading_pane
                        .update_star(account_id, thread_id, *previous);
                }
                OptimisticMutation::SetRead {
                    account_id,
                    thread_id,
                    previous,
                } => {
                    if let Some(t) = self
                        .thread_list
                        .threads
                        .iter_mut()
                        .find(|t| t.account_id == *account_id && t.id == *thread_id)
                    {
                        t.is_read = *previous;
                    }
                }
                OptimisticMutation::SetPinned {
                    account_id,
                    thread_id,
                    previous,
                } => {
                    if let Some(t) = self
                        .thread_list
                        .threads
                        .iter_mut()
                        .find(|t| t.account_id == *account_id && t.id == *thread_id)
                    {
                        t.is_pinned = *previous;
                    }
                }
                OptimisticMutation::SetMuted {
                    account_id,
                    thread_id,
                    previous,
                } => {
                    if let Some(t) = self
                        .thread_list
                        .threads
                        .iter_mut()
                        .find(|t| t.account_id == *account_id && t.id == *thread_id)
                    {
                        t.is_muted = *previous;
                    }
                }
            }
        }
    }

    // ── Unified completion handler (Phase C) ────────────────

    pub(crate) fn handle_action_completed(
        &mut self,
        plan: &crate::action_resolve::ActionExecutionPlan,
        outcomes: &[ActionOutcome],
    ) -> Task<Message> {
        use crate::action_resolve::{self as ar, PostSuccessEffect, UndoBehavior, ViewEffect};

        let behavior = &plan.behavior;

        // Outcome summary
        let all_noop = outcomes.iter().all(ActionOutcome::is_noop);
        let all_failed = outcomes.iter().all(ActionOutcome::is_failed);
        let any_failed = outcomes.iter().any(ActionOutcome::is_failed);

        if all_noop {
            return Task::none();
        }

        // All failed → show detailed error, rollback toggles, don't advance
        if all_failed {
            let errors: Vec<String> = outcomes
                .iter()
                .filter_map(|o| match o {
                    ActionOutcome::Failed { error } => Some(error.user_message()),
                    _ => None,
                })
                .collect();
            self.status_bar.show_confirmation(format!(
                "\u{26A0} {} failed: {}",
                behavior.success_label,
                errors.join("; ")
            ));
            // Rollback all optimistic toggle mutations on total failure
            if !plan.optimistic.is_empty() {
                self.rollback_optimistic(&plan.optimistic);
            }
            return Task::none();
        }

        // Show toast - all text policy centralized in format_outcome_toast (C3/D3)
        let toast = ar::format_outcome_toast(behavior, outcomes);
        if !toast.is_empty() {
            self.status_bar.show_confirmation(toast);
        }

        // Rollback failed toggle optimistic mutations
        if any_failed && !plan.optimistic.is_empty() {
            let failed_mutations: Vec<_> = plan
                .optimistic
                .iter()
                .zip(outcomes.iter())
                .filter(|(_, o)| o.is_failed())
                .map(|(m, _)| m.clone())
                .collect();
            if all_failed {
                self.rollback_optimistic(&plan.optimistic);
            } else {
                self.rollback_optimistic(&failed_mutations);
            }
        }

        // Build and push undo payloads (C2, C4, C5)
        if matches!(behavior.undo, UndoBehavior::Reversible) {
            let payloads = ar::build_undo_payloads(plan, outcomes);
            if !payloads.is_empty() {
                let desc = ar::undo_description(&payloads);
                self.undo_stack.push(desc, payloads);
            }
        }

        // Post-success effects
        match behavior.view_effect {
            ViewEffect::LeavesCurrentView if !all_failed => {
                return self
                    .handle_thread_list(crate::ui::thread_list::ThreadListMessage::AutoAdvance);
            }
            _ => {}
        }
        match behavior.post_success {
            PostSuccessEffect::RefreshNav if !all_failed => {
                let token = self.nav_generation.next();
                return self.fire_navigation_load(token);
            }
            _ => {}
        }

        Task::none()
    }

    // ── Undo dispatch ───────────────────────────────────────

    pub(crate) fn dispatch_undo(
        &mut self,
        entry: cmdk::UndoEntry<crate::action_resolve::MailUndoPayload>,
    ) -> Task<Message> {
        let Some(mut ctx) = self.action_ctx() else {
            self.status_bar.show_confirmation(
                "\u{26A0} Undo unavailable \u{2014} action service not initialized".to_string(),
            );
            return Task::none();
        };
        ctx.suppress_pending_enqueue = true;

        let desc = entry.description.clone();
        Task::perform(
            async move {
                let mut all_outcomes = Vec::new();
                // C8: payload execution order is stored entry order
                for payload in &entry.payloads {
                    all_outcomes.extend(execute_undo_compensation(&ctx, payload).await);
                }
                (desc, all_outcomes)
            },
            |(desc, outcomes)| Message::UndoCompleted { desc, outcomes },
        )
    }
}

// ── Undo compensation ───────────────────────────────────────────────

/// Execute undo compensation for a single payload.
async fn execute_undo_compensation(
    ctx: &rtsk::actions::ActionContext,
    payload: &crate::action_resolve::MailUndoPayload,
) -> Vec<ActionOutcome> {
    use crate::action_resolve::MailUndoPayload;
    use rtsk::actions;
    use rtsk::db::pending_ops::db_pending_ops_cancel_for_resource;

    match payload {
        MailUndoPayload::Archive {
            account_id,
            thread_ids,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            let inbox = TagId::from("INBOX");
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "archive".to_string(),
                )
                .await;
                outcomes.push(actions::add_label(ctx, account_id, tid, &inbox).await);
            }
            outcomes
        }
        MailUndoPayload::Trash {
            account_id,
            thread_ids,
            source,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "trash".to_string(),
                )
                .await;
                let outcome = if let Some(folder) = source {
                    actions::move_to_folder(ctx, account_id, tid, folder, None).await
                } else {
                    let inbox = TagId::from("INBOX");
                    actions::add_label(ctx, account_id, tid, &inbox).await
                };
                outcomes.push(outcome);
            }
            outcomes
        }
        MailUndoPayload::MoveToFolder {
            account_id,
            thread_ids,
            source,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "moveToFolder".to_string(),
                )
                .await;
                outcomes.push(actions::move_to_folder(ctx, account_id, tid, source, None).await);
            }
            outcomes
        }
        MailUndoPayload::SetSpam {
            account_id,
            thread_ids,
            was_spam,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "spam".to_string(),
                )
                .await;
                outcomes.push(actions::spam(ctx, account_id, tid, *was_spam).await);
            }
            outcomes
        }
        MailUndoPayload::SetRead {
            account_id,
            thread_ids,
            was_read,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "markRead".to_string(),
                )
                .await;
                outcomes.push(actions::mark_read(ctx, account_id, tid, *was_read).await);
            }
            outcomes
        }
        MailUndoPayload::SetStarred {
            account_id,
            thread_ids,
            was_starred,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "star".to_string(),
                )
                .await;
                outcomes.push(actions::star(ctx, account_id, tid, *was_starred).await);
            }
            outcomes
        }
        MailUndoPayload::SetPinned {
            account_id,
            thread_ids,
            was_pinned,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                outcomes.push(actions::pin(ctx, account_id, tid, *was_pinned).await);
            }
            outcomes
        }
        MailUndoPayload::SetMuted {
            account_id,
            thread_ids,
            was_muted,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                outcomes.push(actions::mute(ctx, account_id, tid, *was_muted).await);
            }
            outcomes
        }
        MailUndoPayload::AddLabel {
            account_id,
            thread_ids,
            label_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "addLabel".to_string(),
                )
                .await;
                outcomes.push(actions::remove_label(ctx, account_id, tid, label_id).await);
            }
            outcomes
        }
        MailUndoPayload::RemoveLabel {
            account_id,
            thread_ids,
            label_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db,
                    account_id.clone(),
                    tid.clone(),
                    "removeLabel".to_string(),
                )
                .await;
                outcomes.push(actions::add_label(ctx, account_id, tid, label_id).await);
            }
            outcomes
        }
        MailUndoPayload::Snooze {
            account_id,
            thread_ids,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                outcomes.push(actions::unsnooze(ctx, account_id, tid).await);
            }
            outcomes
        }
    }
}

// ── Snooze resurface ─────────────────────────────────────────

// ── Snooze resurface ─────────────────────────────────────────

impl App {
    /// Check for snoozed threads that are due and unsnooze them.
    pub(crate) fn handle_snooze_tick(&self) -> Task<Message> {
        let Some(ctx) = self.action_ctx() else {
            return Task::none();
        };
        Task::perform(
            async move {
                let now = chrono::Utc::now().timestamp();
                let due = rtsk::db::queries_extra::db_get_snoozed_threads_due(&ctx.db, now).await?;
                if due.is_empty() {
                    return Ok(0);
                }
                let mut success_count = 0usize;
                for thread in &due {
                    let outcome =
                        rtsk::actions::unsnooze(&ctx, &thread.account_id, &thread.id).await;
                    match outcome {
                        rtsk::actions::ActionOutcome::Success => {
                            success_count += 1;
                        }
                        rtsk::actions::ActionOutcome::Failed { error } => {
                            log::error!(
                                "Failed to unsnooze thread {}: {}",
                                thread.id,
                                error.user_message()
                            );
                        }
                        _ => {}
                    }
                }
                if success_count > 0 {
                    log::info!("Snooze resurface: unsnoozed {success_count} thread(s)");
                }
                Ok(success_count)
            },
            Message::SnoozeResurfaceComplete,
        )
    }

    /// After unsnoozing due threads, reload navigation (unread counts) and thread list.
    // NOTE: The methods above this line are the Phase A boundary.
    // Below: snooze tick (unchanged) + adapter functions.
    pub(crate) fn handle_snooze_resurface_complete(
        &mut self,
        result: Result<usize, String>,
    ) -> Task<Message> {
        match result {
            Ok(0) => Task::none(),
            Ok(_count) => self.load_navigation_and_threads(),
            Err(e) => {
                log::error!("Snooze resurface check failed: {e}");
                Task::none()
            }
        }
    }
}

// ── Command argument helpers ─────────────────────────────────────────

/// Build `CommandArgs` from a palette option selection.
pub(crate) fn build_command_args(command_id: CommandId, item: &OptionItem) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => Some(CommandArgs::MoveToFolder {
            folder_id: item.id.clone().into(),
        }),
        CommandId::EmailAddLabel => Some(CommandArgs::AddLabel {
            label_id: item.id.clone().into(),
        }),
        CommandId::EmailRemoveLabel => Some(CommandArgs::RemoveLabel {
            label_id: item.id.clone().into(),
        }),
        CommandId::EmailSnooze => item
            .id
            .parse::<i64>()
            .ok()
            .map(|ts| CommandArgs::Snooze { until: ts }),
        CommandId::NavigateToLabel => {
            let (account_id, is_tag, label_id) = split_cross_account_id(&item.id)?;
            if is_tag {
                Some(CommandArgs::NavigateToTag {
                    tag_id: label_id.into(),
                    account_id,
                })
            } else {
                Some(CommandArgs::NavigateToFolder {
                    folder_id: label_id.into(),
                    account_id,
                })
            }
        }
        _ => None,
    }
}

/// Build `CommandArgs` from free text input for Text-param commands.
pub(crate) fn build_command_args_from_text(
    command_id: CommandId,
    text: &str,
) -> Option<CommandArgs> {
    match command_id {
        CommandId::SmartFolderSave => Some(CommandArgs::SmartFolderSave {
            name: text.to_string(),
        }),
        _ => None,
    }
}

/// Split a cross-account encoded ID ("account_id:kind:label_id") into its parts.
/// `kind` is "t" for tag, "f" for folder.
fn split_cross_account_id(encoded: &str) -> Option<(String, bool, String)> {
    let mut parts = encoded.splitn(3, ':');
    let account_id = parts.next()?.to_string();
    let kind = parts.next()?;
    let label_id = parts.next()?.to_string();
    if account_id.is_empty() || label_id.is_empty() {
        return None;
    }
    Some((account_id, kind == "t", label_id))
}
