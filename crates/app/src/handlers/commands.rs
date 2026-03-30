use iced::Task;

use std::sync::Arc;

use crate::command_dispatch::{self, EmailAction};
use crate::db::Db;
use crate::{APP_DATA_DIR, App, CompletedAction, Message};
use ratatoskr_command_palette::{CommandArgs, CommandId, KeyBinding, OptionItem};
use ratatoskr_core::actions::{ActionOutcome, FolderId, MailOperation, TagId};
use ratatoskr_core::scope::ViewScope;

/// Parameters for actions that need more than account_id + thread_id.
/// Also used by undo token construction to recover prior state.
#[derive(Debug, Clone)]
pub(crate) enum ActionParams {
    None,
    Spam { is_spam: bool },
    MoveToFolder { folder_id: FolderId, source_label_id: Option<FolderId> },
    Label { label_id: TagId },
    Trash { source_label_id: Option<FolderId> },
    Snooze { until: i64 },
}

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
    /// is not initialized (degraded mode — stores failed at boot).
    pub(crate) fn action_ctx(&self) -> Option<ratatoskr_core::actions::ActionContext> {
        self.action_ctx.as_ref().cloned()
    }

    pub(crate) fn handle_email_action(
        &mut self,
        action: EmailAction,
    ) -> Task<Message> {
        // Public folder items are not real threads — actions don't apply.
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
        use crate::action_resolve::{
            self as ar, MailActionIntent, OptimisticMutation, ResolveOutcome, UiContext,
        };

        let intent = email_action_to_intent(action);
        let ui_ctx = UiContext {
            selected_label: self.sidebar.selected_label.clone(),
        };
        let outcome = ar::resolve_intent(intent, &ui_ctx);

        let Some(plan) = ar::build_execution_plan(
            outcome,
            &selected_threads,
            &mut self.thread_list,
        ) else {
            // NoOp (Unsubscribe)
            self.status_bar.show_confirmation("Unsubscribed".to_string());
            return Task::none();
        };

        // Star optimistic UI → sync reading pane
        if plan.optimistic.iter().any(|m| matches!(m, OptimisticMutation::SetStarred { .. })) {
            for m in &plan.optimistic {
                if let OptimisticMutation::SetStarred { thread_id, previous, .. } = m {
                    self.reading_pane.update_star(thread_id, !previous);
                }
            }
        }

        self.dispatch_plan(plan)
    }

    /// Dispatch an execution plan through the batch executor.
    /// Derives the legacy CompletedAction from the plan for the completion handler.
    pub(crate) fn dispatch_plan(
        &mut self,
        plan: crate::action_resolve::ActionExecutionPlan,
    ) -> Task<Message> {
        let Some(ctx) = self.action_ctx() else {
            // I4: rollback optimistic mutations before returning
            self.rollback_optimistic(&plan.optimistic);
            self.status_bar.show_confirmation(
                "\u{26A0} Action unavailable \u{2014} service not initialized".to_string(),
            );
            return Task::none();
        };

        debug_assert!(!plan.operations.is_empty(), "I1: non-empty plan");

        // I3: derive legacy classifier from first operation — all ops must map to same classifier
        let action = completed_action_from_operation(&plan.operations[0].2);
        debug_assert!(
            plan.operations.iter().all(|(_, _, op)| completed_action_from_operation(op) == action),
            "I3: all operations must map to the same CompletedAction"
        );

        // Extract what we need for the async task + completion message
        let operations = plan.operations.clone();
        let threads: Vec<(String, String)> = plan.operations.iter()
            .map(|(a, t, _)| (a.clone(), t.clone()))
            .collect();
        let rollback: Vec<(String, String, bool)> = plan.optimistic.iter()
            .map(|m| match m {
                crate::action_resolve::OptimisticMutation::SetStarred { account_id, thread_id, previous }
                | crate::action_resolve::OptimisticMutation::SetRead { account_id, thread_id, previous }
                | crate::action_resolve::OptimisticMutation::SetPinned { account_id, thread_id, previous }
                | crate::action_resolve::OptimisticMutation::SetMuted { account_id, thread_id, previous } => {
                    (account_id.clone(), thread_id.clone(), *previous)
                }
            })
            .collect();
        let params = action_params_from_plan(&plan, action);

        Task::perform(
            async move {
                let outcomes = ratatoskr_core::actions::batch_execute(&ctx, operations).await;
                (action, outcomes, threads)
            },
            move |(action, outcomes, threads)| Message::ActionCompleted {
                action,
                outcomes,
                rollback,
                threads,
                params,
            },
        )
    }

    /// Rollback optimistic mutations when dispatch cannot proceed (I4).
    fn rollback_optimistic(&mut self, mutations: &[crate::action_resolve::OptimisticMutation]) {
        use crate::action_resolve::OptimisticMutation;
        for m in mutations {
            match m {
                OptimisticMutation::SetStarred { account_id, thread_id, previous } => {
                    if let Some(t) = self.thread_list.threads.iter_mut().find(
                        |t| t.account_id == *account_id && t.id == *thread_id,
                    ) {
                        t.is_starred = *previous;
                    }
                    self.reading_pane.update_star(thread_id, *previous);
                }
                OptimisticMutation::SetRead { account_id, thread_id, previous } => {
                    if let Some(t) = self.thread_list.threads.iter_mut().find(
                        |t| t.account_id == *account_id && t.id == *thread_id,
                    ) {
                        t.is_read = *previous;
                    }
                }
                OptimisticMutation::SetPinned { account_id, thread_id, previous } => {
                    if let Some(t) = self.thread_list.threads.iter_mut().find(
                        |t| t.account_id == *account_id && t.id == *thread_id,
                    ) {
                        t.is_pinned = *previous;
                    }
                }
                OptimisticMutation::SetMuted { account_id, thread_id, previous } => {
                    if let Some(t) = self.thread_list.threads.iter_mut().find(
                        |t| t.account_id == *account_id && t.id == *thread_id,
                    ) {
                        t.is_muted = *previous;
                    }
                }
            }
        }
    }

    /// Sync reading pane state after a thread list toggle or rollback.
    /// Ensures the reading pane reflects the same state as the thread list
    /// for any toggle that has a reading pane counterpart.
    fn sync_reading_pane_after_toggle(
        &mut self,
        action: CompletedAction,
        threads: &[(String, String, bool)],
        use_new_value: bool,
    ) {
        if matches!(action, CompletedAction::Star) {
            for (_, tid, stored_val) in threads {
                let star_val = if use_new_value { !stored_val } else { *stored_val };
                self.reading_pane.update_star(tid, star_val);
            }
        }
    }

    /// Restore previous toggle values on failure. Finds threads by ID, not index.
    fn rollback_toggles(
        &mut self,
        rollback: &[(String, String, bool)],
        action: CompletedAction,
    ) {
        for (account_id, thread_id, prev) in rollback {
            // Restore thread list state if the thread is still present.
            if let Some(t) = self.thread_list.threads.iter_mut().find(
                |t| t.account_id == *account_id && t.id == *thread_id,
            ) {
                match action {
                    CompletedAction::Star => t.is_starred = *prev,
                    CompletedAction::MarkRead => t.is_read = *prev,
                    CompletedAction::Pin => t.is_pinned = *prev,
                    CompletedAction::Mute => t.is_muted = *prev,
                    // Non-toggle actions don't have thread-list rollback fields.
                    CompletedAction::Archive
                    | CompletedAction::Trash
                    | CompletedAction::Spam
                    | CompletedAction::MoveToFolder
                    | CompletedAction::PermanentDelete
                    | CompletedAction::Snooze
                    | CompletedAction::AddLabel
                    | CompletedAction::RemoveLabel => {}
                }
            }

        }
        // Sync reading pane after rollback (restoring old values)
        self.sync_reading_pane_after_toggle(action, rollback, false);
    }

    /// Handle action service completion — map outcomes to user feedback,
    /// auto-advance for removes-from-view, rollback for failed toggles.
    pub(crate) fn handle_action_completed(
        &mut self,
        action: CompletedAction,
        outcomes: &[ActionOutcome],
        rollback: &[(String, String, bool)],
        threads: &[(String, String)],
        params: &ActionParams,
    ) -> Task<Message> {
        let all_failed = outcomes.iter().all(ActionOutcome::is_failed);
        let all_noop = outcomes.iter().all(|o| matches!(o, ActionOutcome::NoOp));
        if all_noop {
            return Task::none();
        }
        let any_failed = outcomes.iter().any(ActionOutcome::is_failed);
        let any_local_only = outcomes.iter().any(ActionOutcome::is_local_only);

        if action.removes_from_view() {
            if all_failed {
                let errors: Vec<String> = outcomes
                    .iter()
                    .filter_map(|o| match o {
                        ActionOutcome::Failed { error } => Some(error.user_message()),
                        _ => None,
                    })
                    .collect();
                // TODO: use dedicated error display once status bar supports it.
                self.status_bar.show_confirmation(
                    format!("\u{26A0} {} failed: {}", action.success_label(), errors.join("; ")),
                );
                return Task::none();
            }

            if any_failed {
                // Mixed: some succeeded, some failed.
                let succeeded = outcomes.iter().filter(|o| !o.is_failed()).count();
                let total = outcomes.len();
                // TODO: use dedicated warning display once status bar supports it.
                self.status_bar.show_confirmation(
                    format!("\u{26A0} {} {succeeded} of {total} threads \u{2014} {failed} failed",
                        action.success_label(), failed = total - succeeded),
                );
            } else if any_local_only {
                // TODO: use dedicated warning display once status bar supports it.
                self.status_bar.show_confirmation(
                    format!("\u{26A0} {} locally \u{2014} sync may revert this", action.success_label()),
                );
            } else {
                self.status_bar
                    .show_confirmation(action.success_label().to_string());
            }

            // Produce undo tokens for succeeded threads
            self.produce_undo_tokens(action, outcomes, threads, rollback, params);

            return self.handle_thread_list(
                crate::ui::thread_list::ThreadListMessage::AutoAdvance,
            );
        }

        // ── Non-toggle, non-removes-from-view actions (labels) ──
        // Toggle actions have rollback data; their optimistic UI IS the feedback.
        // Label-type actions have no rollback and need an explicit toast.
        if rollback.is_empty() {
            if all_failed {
                let errors: Vec<String> = outcomes
                    .iter()
                    .filter_map(|o| match o {
                        ActionOutcome::Failed { error } => Some(error.user_message()),
                        _ => None,
                    })
                    .collect();
                self.status_bar.show_confirmation(
                    format!("\u{26A0} {} failed: {}", action.success_label(), errors.join("; ")),
                );
            } else if any_failed {
                let succeeded = outcomes.iter().filter(|o| !o.is_failed()).count();
                let total = outcomes.len();
                self.status_bar.show_confirmation(
                    format!(
                        "\u{26A0} {} {succeeded} of {total} threads \u{2014} {} failed",
                        action.success_label(),
                        total - succeeded
                    ),
                );
            } else if any_local_only {
                self.status_bar.show_confirmation(
                    format!(
                        "\u{26A0} {} locally \u{2014} sync may revert this",
                        action.success_label()
                    ),
                );
            } else {
                self.status_bar
                    .show_confirmation(action.success_label().to_string());
            }
            // Produce undo tokens for succeeded threads (labels)
            self.produce_undo_tokens(action, outcomes, threads, rollback, params);
            return Task::none();
        }

        // Toggle actions: rollback failed threads, refresh nav for read status.
        if all_failed {
            self.rollback_toggles(rollback, action);
        } else if any_failed {
            // Mixed: rollback only the failed ones.
            let failed_rollback: Vec<(String, String, bool)> = rollback
                .iter()
                .zip(outcomes.iter())
                .filter(|(_, o)| o.is_failed())
                .map(|(r, _)| r.clone())
                .collect();
            self.rollback_toggles(&failed_rollback, action);
        }

        // Produce undo tokens for succeeded toggle threads
        self.produce_undo_tokens(action, outcomes, threads, rollback, params);

        // Refresh nav state for read status changes (updates unread counts).
        if matches!(action, CompletedAction::MarkRead) && !all_failed {
            let token = self.nav_generation.next();
            return self.fire_navigation_load(token);
        }

        Task::none()
    }

    // ── Undo token production ─────────────────────────────

    /// Produce undo tokens for succeeded threads, grouped by account.
    /// Called by handle_action_completed on non-all-failed outcomes.
    fn produce_undo_tokens(
        &mut self,
        action: CompletedAction,
        outcomes: &[ActionOutcome],
        threads: &[(String, String)],
        rollback: &[(String, String, bool)],
        params: &ActionParams,
    ) {
        use ratatoskr_command_palette::UndoToken;
        use std::collections::HashMap;

        let all_failed = outcomes.iter().all(ActionOutcome::is_failed);
        if all_failed {
            return;
        }

        // PermanentDelete is irreversible — no undo token
        if matches!(action, CompletedAction::PermanentDelete) {
            return;
        }

        if !rollback.is_empty() {
            // Toggle actions: use rollback data (filtered to actually changed).
            // Skip Failed (local didn't apply) and NoOp (state didn't change).
            let mut by_key: HashMap<(&str, bool), Vec<String>> = HashMap::new();
            for ((aid, tid, prev), outcome) in rollback.iter().zip(outcomes.iter()) {
                if outcome.is_success() || outcome.is_local_only() {
                    by_key
                        .entry((aid.as_str(), *prev))
                        .or_default()
                        .push(tid.clone());
                }
            }
            for ((account_id, prev), thread_ids) in by_key {
                let token = match action {
                    CompletedAction::Star => UndoToken::ToggleStar {
                        account_id: account_id.to_string(),
                        thread_ids,
                        was_starred: prev,
                    },
                    CompletedAction::MarkRead => UndoToken::ToggleRead {
                        account_id: account_id.to_string(),
                        thread_ids,
                        was_read: prev,
                    },
                    CompletedAction::Pin => UndoToken::TogglePin {
                        account_id: account_id.to_string(),
                        thread_ids,
                        was_pinned: prev,
                    },
                    CompletedAction::Mute => UndoToken::ToggleMute {
                        account_id: account_id.to_string(),
                        thread_ids,
                        was_muted: prev,
                    },
                    // Non-toggle actions don't enter the rollback branch.
                    CompletedAction::Archive
                    | CompletedAction::Trash
                    | CompletedAction::Spam
                    | CompletedAction::MoveToFolder
                    | CompletedAction::PermanentDelete
                    | CompletedAction::Snooze
                    | CompletedAction::AddLabel
                    | CompletedAction::RemoveLabel => continue,
                };
                self.undo_stack.push(token);
            }
        } else {
            // Non-toggle actions: use threads + outcomes (filtered to non-failed)
            let mut by_account: HashMap<&str, Vec<String>> = HashMap::new();
            for ((aid, tid), outcome) in threads.iter().zip(outcomes.iter()) {
                if outcome.is_success() || outcome.is_local_only() {
                    by_account
                        .entry(aid.as_str())
                        .or_default()
                        .push(tid.clone());
                }
            }
            for (account_id, thread_ids) in by_account {
                let token = match action {
                    CompletedAction::Archive => UndoToken::Archive {
                        account_id: account_id.to_string(),
                        thread_ids,
                    },
                    CompletedAction::Trash => {
                        let source = match params {
                            ActionParams::Trash { source_label_id } => {
                                source_label_id.as_ref().map(|f| f.as_str().to_string())
                            }
                            _ => None,
                        };
                        // No token if source is unknown — incorrect undo
                        // (moving to INBOX) is worse than no undo.
                        let Some(original_folder_id) = source else {
                            continue;
                        };
                        UndoToken::Trash {
                            account_id: account_id.to_string(),
                            thread_ids,
                            original_folder_id: Some(original_folder_id),
                        }
                    }
                    CompletedAction::Spam => {
                        let was_spam = match params {
                            ActionParams::Spam { is_spam } => !is_spam,
                            _ => false,
                        };
                        UndoToken::ToggleSpam {
                            account_id: account_id.to_string(),
                            thread_ids,
                            was_spam,
                        }
                    }
                    CompletedAction::MoveToFolder => {
                        let source = match params {
                            ActionParams::MoveToFolder {
                                source_label_id, ..
                            } => source_label_id.as_ref().map(|f| f.as_str().to_string()),
                            _ => None,
                        };
                        let Some(source_folder_id) = source else {
                            continue; // No source = no undo
                        };
                        UndoToken::MoveToFolder {
                            account_id: account_id.to_string(),
                            thread_ids,
                            source_folder_id,
                        }
                    }
                    CompletedAction::AddLabel => {
                        let label_id = match params {
                            ActionParams::Label { label_id } => label_id.as_str().to_string(),
                            _ => continue,
                        };
                        UndoToken::AddLabel {
                            account_id: account_id.to_string(),
                            thread_ids,
                            label_id,
                        }
                    }
                    CompletedAction::RemoveLabel => {
                        let label_id = match params {
                            ActionParams::Label { label_id } => label_id.as_str().to_string(),
                            _ => continue,
                        };
                        UndoToken::RemoveLabel {
                            account_id: account_id.to_string(),
                            thread_ids,
                            label_id,
                        }
                    }
                    CompletedAction::Snooze => UndoToken::Snooze {
                        account_id: account_id.to_string(),
                        thread_ids,
                    },
                    // Toggle actions are handled in the rollback branch above.
                    // PermanentDelete is irreversible (early return at line 556).
                    CompletedAction::Star
                    | CompletedAction::MarkRead
                    | CompletedAction::Pin
                    | CompletedAction::Mute
                    | CompletedAction::PermanentDelete => continue,
                };
                self.undo_stack.push(token);
            }
        }
    }

    // ── Undo dispatch ─────────────────────────────────────

    /// Dispatch compensation for an undo token through the action service.
    /// Uses suppress_pending_enqueue to prevent re-enqueue during undo.
    /// Bypasses ActionCompleted — returns UndoCompleted directly.
    pub(crate) fn dispatch_undo(
        &mut self,
        token: ratatoskr_command_palette::UndoToken,
    ) -> Task<Message> {
        let Some(mut ctx) = self.action_ctx() else {
            return Task::none();
        };
        ctx.suppress_pending_enqueue = true;
        let desc = token.description();

        Task::perform(
            async move {
                let outcomes = execute_undo_compensation(&ctx, &token).await;
                (desc, outcomes)
            },
            |(desc, outcomes)| Message::UndoCompleted { desc, outcomes },
        )
    }

}

/// Build typed `CommandArgs` from the selected option item.
///
/// Maps each parameterized `CommandId` to its corresponding `CommandArgs`
/// variant, extracting the item's ID (and for cross-account commands,
/// splitting the `"account_id:label_id"` encoding).
pub(crate) fn build_command_args(command_id: CommandId, item: &OptionItem) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => Some(CommandArgs::MoveToFolder {
            folder_id: item.id.clone(),
        }),
        CommandId::EmailAddLabel => Some(CommandArgs::AddLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailRemoveLabel => Some(CommandArgs::RemoveLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailSnooze => {
            // DateTime picker returns a stringified unix timestamp
            item.id
                .parse::<i64>()
                .ok()
                .map(|ts| CommandArgs::Snooze { until: ts })
        }
        CommandId::NavigateToLabel => {
            let (account_id, label_id) = split_cross_account_id(&item.id)?;
            Some(CommandArgs::NavigateToLabel {
                label_id,
                account_id,
            })
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

/// Split a cross-account encoded ID ("account_id:label_id") into its parts.
fn split_cross_account_id(encoded: &str) -> Option<(String, String)> {
    let colon_pos = encoded.find(':')?;
    let account_id = encoded[..colon_pos].to_string();
    let label_id = encoded[colon_pos + 1..].to_string();
    if account_id.is_empty() || label_id.is_empty() {
        return None;
    }
    Some((account_id, label_id))
}

/// Execute compensation actions for an undo token.
/// Cancels pending ops first, then dispatches the inverse action for each thread.
async fn execute_undo_compensation(
    ctx: &ratatoskr_core::actions::ActionContext,
    token: &ratatoskr_command_palette::UndoToken,
) -> Vec<ratatoskr_core::actions::ActionOutcome> {
    use ratatoskr_command_palette::UndoToken;
    use ratatoskr_core::actions;
    use ratatoskr_core::db::pending_ops::db_pending_ops_cancel_for_resource;

    match token {
        UndoToken::Archive {
            account_id,
            thread_ids,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "archive".to_string(),
                ).await;
                let inbox = TagId::from("INBOX");
                outcomes.push(actions::add_label(ctx, account_id, tid, &inbox).await);
            }
            outcomes
        }
        UndoToken::Trash {
            account_id,
            thread_ids,
            original_folder_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "trash".to_string(),
                ).await;
                let outcome = if let Some(folder) = original_folder_id {
                    let fid = FolderId::from(folder.as_str());
                    actions::move_to_folder(ctx, account_id, tid, &fid, None).await
                } else {
                    let inbox = TagId::from("INBOX");
                    actions::add_label(ctx, account_id, tid, &inbox).await
                };
                outcomes.push(outcome);
            }
            outcomes
        }
        UndoToken::MoveToFolder {
            account_id,
            thread_ids,
            source_folder_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            let fid = FolderId::from(source_folder_id.as_str());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "moveToFolder".to_string(),
                ).await;
                outcomes.push(
                    actions::move_to_folder(ctx, account_id, tid, &fid, None).await,
                );
            }
            outcomes
        }
        UndoToken::ToggleRead {
            account_id,
            thread_ids,
            was_read,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "markRead".to_string(),
                ).await;
                outcomes.push(actions::mark_read(ctx, account_id, tid, *was_read).await);
            }
            outcomes
        }
        UndoToken::ToggleStar {
            account_id,
            thread_ids,
            was_starred,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "star".to_string(),
                ).await;
                outcomes.push(actions::star(ctx, account_id, tid, *was_starred).await);
            }
            outcomes
        }
        UndoToken::TogglePin {
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
        UndoToken::ToggleMute {
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
        UndoToken::ToggleSpam {
            account_id,
            thread_ids,
            was_spam,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "spam".to_string(),
                ).await;
                outcomes.push(actions::spam(ctx, account_id, tid, *was_spam).await);
            }
            outcomes
        }
        UndoToken::AddLabel {
            account_id,
            thread_ids,
            label_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            let tid_typed = TagId::from(label_id.as_str());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "addLabel".to_string(),
                ).await;
                outcomes.push(actions::remove_label(ctx, account_id, tid, &tid_typed).await);
            }
            outcomes
        }
        UndoToken::RemoveLabel {
            account_id,
            thread_ids,
            label_id,
        } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            let tid_typed = TagId::from(label_id.as_str());
            for tid in thread_ids {
                let _ = db_pending_ops_cancel_for_resource(
                    &ctx.db, account_id.clone(), tid.clone(), "removeLabel".to_string(),
                ).await;
                outcomes.push(actions::add_label(ctx, account_id, tid, &tid_typed).await);
            }
            outcomes
        }
        UndoToken::Snooze { account_id, thread_ids } => {
            let mut outcomes = Vec::with_capacity(thread_ids.len());
            for tid in thread_ids {
                outcomes.push(
                    ratatoskr_core::actions::unsnooze(ctx, account_id, tid).await,
                );
            }
            outcomes
        }
    }
}

// ── Batch helpers ───────────────────────────────────────────────────

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
                let due = ratatoskr_core::db::queries_extra::db_get_snoozed_threads_due(
                    &ctx.db, now,
                )
                .await?;
                if due.is_empty() {
                    return Ok(0);
                }
                let mut success_count = 0usize;
                for thread in &due {
                    let outcome = ratatoskr_core::actions::unsnooze(
                        &ctx,
                        &thread.account_id,
                        &thread.id,
                    )
                    .await;
                    match outcome {
                        ratatoskr_core::actions::ActionOutcome::Success => {
                            success_count += 1;
                        }
                        ratatoskr_core::actions::ActionOutcome::Failed { error } => {
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

// ── Adapters: EmailAction → MailActionIntent, plan → legacy types ────
//
// email_action_to_intent survives until Phase C (EmailAction is still
// the Message variant type). The plan-to-legacy helpers bridge the new
// plan to the existing completion handler until Phase C replaces it.

use crate::action_resolve::{CompensationContext, MailActionIntent};

/// Convert an `EmailAction` to a `MailActionIntent`.
fn email_action_to_intent(action: EmailAction) -> MailActionIntent {
    match action {
        EmailAction::Archive => MailActionIntent::Archive,
        EmailAction::Trash => MailActionIntent::Trash,
        EmailAction::PermanentDelete => MailActionIntent::PermanentDelete,
        EmailAction::ToggleSpam => MailActionIntent::ToggleSpam,
        EmailAction::ToggleStar => MailActionIntent::ToggleStar,
        EmailAction::ToggleRead => MailActionIntent::ToggleRead,
        EmailAction::TogglePin => MailActionIntent::TogglePin,
        EmailAction::ToggleMute => MailActionIntent::ToggleMute,
        EmailAction::Unsubscribe => MailActionIntent::Unsubscribe,
        EmailAction::MoveToFolder { folder_id } => MailActionIntent::MoveToFolder { folder_id },
        EmailAction::AddLabel { label_id } => MailActionIntent::AddLabel { label_id },
        EmailAction::RemoveLabel { label_id } => MailActionIntent::RemoveLabel { label_id },
        EmailAction::Snooze { until } => MailActionIntent::Snooze { until },
    }
}

/// Derive `CompletedAction` from the first operation in a plan (I3).
fn completed_action_from_operation(op: &MailOperation) -> CompletedAction {
    match op {
        MailOperation::Archive => CompletedAction::Archive,
        MailOperation::Trash => CompletedAction::Trash,
        MailOperation::PermanentDelete => CompletedAction::PermanentDelete,
        MailOperation::SetSpam { .. } => CompletedAction::Spam,
        MailOperation::MoveToFolder { .. } => CompletedAction::MoveToFolder,
        MailOperation::SetStarred { .. } => CompletedAction::Star,
        MailOperation::SetRead { .. } => CompletedAction::MarkRead,
        MailOperation::SetPinned { .. } => CompletedAction::Pin,
        MailOperation::SetMuted { .. } => CompletedAction::Mute,
        MailOperation::AddLabel { .. } => CompletedAction::AddLabel,
        MailOperation::RemoveLabel { .. } => CompletedAction::RemoveLabel,
        MailOperation::Snooze { .. } => CompletedAction::Snooze,
    }
}

/// Derive legacy `ActionParams` from a plan + completed action.
/// Bridges the new plan to the existing completion/undo handler.
fn action_params_from_plan(
    plan: &crate::action_resolve::ActionExecutionPlan,
    action: CompletedAction,
) -> ActionParams {
    match action {
        CompletedAction::Spam => {
            let is_spam = match &plan.operations[0].2 {
                MailOperation::SetSpam { to } => *to,
                _ => true,
            };
            ActionParams::Spam { is_spam }
        }
        CompletedAction::Trash => {
            let source = match &plan.compensation {
                CompensationContext::SourceFolder(s) => s.clone(),
                CompensationContext::None => None,
            };
            ActionParams::Trash { source_label_id: source }
        }
        CompletedAction::MoveToFolder => {
            let (folder_id, source_label_id) = match &plan.operations[0].2 {
                MailOperation::MoveToFolder { dest, source } => {
                    (dest.clone(), source.clone())
                }
                _ => (FolderId::from(""), None),
            };
            ActionParams::MoveToFolder { folder_id, source_label_id }
        }
        CompletedAction::AddLabel | CompletedAction::RemoveLabel => {
            let label_id = match &plan.operations[0].2 {
                MailOperation::AddLabel { label_id } | MailOperation::RemoveLabel { label_id } => {
                    label_id.clone()
                }
                _ => TagId::from(""),
            };
            ActionParams::Label { label_id }
        }
        CompletedAction::Snooze => {
            let until = match &plan.operations[0].2 {
                MailOperation::Snooze { until } => *until,
                _ => 0,
            };
            ActionParams::Snooze { until }
        }
        _ => ActionParams::None,
    }
}
