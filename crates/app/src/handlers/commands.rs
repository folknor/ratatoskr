use std::sync::Arc;

use iced::Task;

use crate::command_dispatch;
use crate::{APP_DATA_DIR, Message, ReadyApp};
use cmdk::{CommandArgs, CommandId, KeyBinding, OptionItem};
use rtsk::actions::{ActionOutcome, TagId};
use rtsk::scope::ViewScope;

#[allow(dead_code)] // Keybinding override API; not yet wired into the settings UI.
impl ReadyApp {
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

    /// Dispatch an execution plan via IPC to the Service-side action
    /// service. The Service journals the plan into `action_jobs` /
    /// `action_job_ops`, returns `ActionPlanAck`, then the worker
    /// drives execution and streams `OperationOutcome` /
    /// `ActionCompleted` notifications back. Notifications hit
    /// `Message::ServiceNotification` -> the per-plan outcomes
    /// accumulate in `ReadyApp.pending_action_plans`, and
    /// `Notification::ActionCompleted` fires `Message::ActionCompleted`
    /// so the existing `handle_action_completed` flow runs unchanged.
    ///
    /// Pre-dispatch:
    /// - Generation counters bump BEFORE the IPC call so a stale
    ///   `ThreadsLoaded` / `NavigationLoaded` landing during the
    ///   round-trip cannot overwrite the optimistic state.
    /// - The plan is stashed in `pending_action_plans` keyed by the
    ///   freshly-generated `PlanId` so notifications can find it.
    pub(crate) fn dispatch_plan(
        &mut self,
        plan: crate::action_resolve::ActionExecutionPlan,
    ) -> Task<Message> {
        self.dispatch_plan_with_undo(plan, None)
    }

    /// Variant of `dispatch_plan` that marks the plan as the inverse
    /// of a previously-completed action. The completion handler fires
    /// `Message::UndoCompleted` (toast + nav + thread-list reload)
    /// instead of `Message::ActionCompleted` (per-behavior
    /// post-success effects), preserving the pre-Phase-2 undo UX.
    pub(crate) fn dispatch_plan_with_undo(
        &mut self,
        plan: crate::action_resolve::ActionExecutionPlan,
        undo_description: Option<String>,
    ) -> Task<Message> {
        if plan.operations.is_empty() {
            return Task::none();
        }

        let Some(client) = self.service_client.as_ref().cloned() else {
            // Pre-Ready (no client) or Service crashed without respawn:
            // roll back optimistic state and surface a toast.
            self.rollback_optimistic(&plan.optimistic);
            self.status_bar.show_confirmation(
                "\u{26A0} Action unavailable \u{2014} service not connected".to_string(),
            );
            return Task::none();
        };

        // Client-side action throttle (Phase 2 plan scope item 12).
        // Absorbs fast double-clicks before they hit the wire. Entries
        // expire when `ActionCompleted` arrives, OR after 200 ms as the
        // safety valve for dropped notifications. Same target inside
        // the window -> drop and roll back optimistic state silently
        // (the user cannot perceive the difference between "the click
        // didn't take" and "the click took but is mid-IPC", and the
        // first click's outcome will land normally).
        let now = std::time::Instant::now();
        let throttle_window = std::time::Duration::from_millis(200);
        self.action_throttle
            .retain(|_, t| now.duration_since(*t) < throttle_window);
        let throttled = plan.operations.iter().any(|(account_id, thread_id, _)| {
            self.action_throttle
                .contains_key(&(account_id.clone(), thread_id.clone()))
        });
        if throttled {
            log::debug!(
                "dispatch_plan dropped: target inside 200ms throttle window (double-click absorbed)",
            );
            self.rollback_optimistic(&plan.optimistic);
            return Task::none();
        }

        // Account-level reconciliation gate (Phase 2 plan scope item 14):
        // if any plan touching one of these accounts is in `AckUnknown`
        // state, dispatching now would let optimistic state pile on top
        // of unresolved state. Surface a toast and bail; the user can
        // retry once reconciliation finishes (the post-respawn
        // `action.job_status` round-trip resolves AckUnknown plans).
        let touched: std::collections::HashSet<&str> = plan
            .operations
            .iter()
            .map(|(account_id, _, _)| account_id.as_str())
            .collect();
        let blocking: Vec<service_api::PlanId> = self
            .pending_action_plans
            .iter()
            .filter(|(_, p)| matches!(p.state, crate::app::PlanState::AckUnknown))
            .filter(|(_, p)| {
                p.plan
                    .operations
                    .iter()
                    .any(|(acct, _, _)| touched.contains(acct.as_str()))
            })
            .map(|(id, _)| *id)
            .collect();
        if !blocking.is_empty() {
            log::warn!(
                "dispatch_plan blocked: {} plan(s) on these accounts pending reconciliation",
                blocking.len(),
            );
            self.rollback_optimistic(&plan.optimistic);
            self.status_bar.show_confirmation(
                "\u{26A0} Action deferred \u{2014} reconciling previous action".to_string(),
            );
            return Task::none();
        }

        // Pre-dispatch invalidation - PRE-IPC, not post-completion.
        // Without these bumps, a stale ThreadsLoaded landing between
        // dispatch and OperationOutcome would overwrite the optimistic
        // update.
        let _ = self.nav_generation.next();
        let _ = self.thread_generation.next();

        // Mark each (account, thread) as recently dispatched so a
        // double-click within 200 ms is dropped at the throttle gate.
        for (account_id, thread_id, _) in &plan.operations {
            self.action_throttle
                .insert((account_id.clone(), thread_id.clone()), now);
        }

        let plan_id = service_api::PlanId::new_v7();
        let wire_plan = crate::action_wire::to_wire_plan(plan_id, &plan);
        self.pending_action_plans.insert(
            plan_id,
            crate::app::PendingActionPlan {
                plan,
                outcomes: Vec::new(),
                state: crate::app::PlanState::Pending,
                applied_outcomes: std::collections::HashSet::new(),
                undo_description,
            },
        );

        Task::perform(
            async move {
                let result = client.execute_plan(wire_plan).await;
                crate::service_client::classify_dispatch(result)
            },
            move |outcome| Message::ActionDispatched { plan_id, outcome },
        )
    }

    /// Synchronous response to `dispatch_plan`'s IPC call.
    ///
    /// Drives the tri-state per Phase 2 plan scope item 14:
    /// - `Acked`: transition `Pending -> Acked`. Plan is durable; do
    ///   nothing else - notifications will drive completion.
    /// - `AckUnknown`: transition `Pending -> AckUnknown`. Hold
    ///   optimistic state. The post-`boot.ready` reconciliation flow
    ///   (`handle_post_respawn_reconcile`) will fire
    ///   `action.job_status` and resolve to either Acked or rollback.
    /// - `Failed`: roll back optimistic state and remove the plan
    ///   from `pending_action_plans`. The Service either rejected the
    ///   plan or the request never went out.
    pub(crate) fn handle_action_dispatched(
        &mut self,
        plan_id: service_api::PlanId,
        outcome: crate::service_client::DispatchOutcome,
    ) -> Task<Message> {
        use crate::service_client::DispatchOutcome;
        match outcome {
            DispatchOutcome::Acked(_ack) => {
                if let Some(state) = self.pending_action_plans.get_mut(&plan_id) {
                    state.state = crate::app::PlanState::Acked;
                } else {
                    // Plan disappeared between dispatch and ack - the
                    // ActionCompleted notification raced ahead (plan
                    // already drained). Nothing to do.
                    log::debug!(
                        "ActionDispatched(Acked) for plan {plan_id:?} - already drained",
                    );
                }
                Task::none()
            }
            DispatchOutcome::AckUnknown { reason } => {
                if let Some(state) = self.pending_action_plans.get_mut(&plan_id) {
                    state.state = crate::app::PlanState::AckUnknown;
                    log::warn!(
                        "action plan {plan_id} entered AckUnknown ({reason}); awaiting reconciliation",
                    );
                } else {
                    log::debug!(
                        "ActionDispatched(AckUnknown) for plan {plan_id:?} - already drained",
                    );
                }
                Task::none()
            }
            DispatchOutcome::Failed { reason } => {
                let pending = self.pending_action_plans.remove(&plan_id);
                if let Some(state) = pending {
                    for (account_id, thread_id, _) in &state.plan.operations {
                        self.action_throttle
                            .remove(&(account_id.clone(), thread_id.clone()));
                    }
                    self.rollback_optimistic(&state.plan.optimistic);
                }
                log::warn!("action plan {plan_id} dispatch failed: {reason}");
                self.status_bar
                    .show_confirmation(format!("\u{26A0} Action failed: {reason}"));
                Task::none()
            }
        }
    }

    /// Fire one `action.job_status` query per plan in `AckUnknown`
    /// state (Phase 2 plan scope item 11 / 18d).
    ///
    /// Triggered by `Message::ServiceBootReady` after every respawn
    /// (initial + every subsequent). On the very first boot there are
    /// no `AckUnknown` plans (nothing has been dispatched yet), so the
    /// returned task is `Task::none()` - the reconciliation only fires
    /// on actual respawns. Per-plan response lands as
    /// `Message::JobStatusResolved` and feeds
    /// `handle_job_status_resolved`, which drains optimistic state per
    /// the response.
    pub(crate) fn kickoff_post_respawn_reconcile(&mut self) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::none();
        };
        let plan_ids: Vec<service_api::PlanId> = self
            .pending_action_plans
            .iter()
            .filter(|(_, p)| matches!(p.state, crate::app::PlanState::AckUnknown))
            .map(|(id, _)| *id)
            .collect();
        if plan_ids.is_empty() {
            return Task::none();
        }
        log::info!(
            "post-respawn reconcile: querying action.job_status for {} AckUnknown plan(s)",
            plan_ids.len(),
        );
        let tasks: Vec<Task<Message>> = plan_ids
            .into_iter()
            .map(|plan_id| {
                let client = Arc::clone(&client);
                Task::perform(
                    async move {
                        client
                            .job_status(plan_id)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |result| Message::JobStatusResolved { plan_id, result },
                )
            })
            .collect();
        Task::batch(tasks)
    }

    /// Resolve one `AckUnknown` plan via the `action.job_status`
    /// response.
    ///
    /// - `Journaled`: the Service has the plan; transition to `Acked`
    ///   and let the worker's outcome replay drive completion.
    /// - `NotFound`: the Service never journaled the plan (it crashed
    ///   before commit, or the request was dropped); roll back the
    ///   optimistic state and remove the plan.
    /// - `Err`: leave the plan in `AckUnknown`. The next respawn will
    ///   retry the query.
    pub(crate) fn handle_job_status_resolved(
        &mut self,
        plan_id: service_api::PlanId,
        result: Result<service_api::JobStatusResponse, String>,
    ) -> Task<Message> {
        match result {
            Ok(service_api::JobStatusResponse::Journaled { status, .. }) => {
                log::info!(
                    "reconcile plan {plan_id}: Journaled (status={status:?}); promoting to Acked",
                );
                if let Some(state) = self.pending_action_plans.get_mut(&plan_id) {
                    state.state = crate::app::PlanState::Acked;
                }
                Task::none()
            }
            Ok(service_api::JobStatusResponse::NotFound) => {
                log::info!("reconcile plan {plan_id}: NotFound; rolling back optimistic state");
                if let Some(state) = self.pending_action_plans.remove(&plan_id) {
                    for (account_id, thread_id, _) in &state.plan.operations {
                        self.action_throttle
                            .remove(&(account_id.clone(), thread_id.clone()));
                    }
                    self.rollback_optimistic(&state.plan.optimistic);
                }
                self.status_bar.show_confirmation(
                    "\u{26A0} Action lost during service restart \u{2014} reverted".to_string(),
                );
                Task::none()
            }
            Err(error) => {
                log::warn!(
                    "reconcile plan {plan_id}: query failed ({error}); will retry on next respawn",
                );
                Task::none()
            }
        }
    }

    /// Wire `OperationOutcome` notification arrival to the per-plan
    /// outcomes accumulator. The companion `ActionCompleted`
    /// notification (handled in `handle_notification_action_completed`)
    /// drains the accumulator and fires `Message::ActionCompleted`.
    ///
    /// Idempotency contract per Phase 2 plan scope item 17: replay
    /// from the journal can re-emit an outcome the UI already saw
    /// (post-respawn the worker drains every `action_job_ops` row
    /// whose `outcome IS NOT NULL` for any non-terminal job). The
    /// `applied_outcomes` set drops the duplicate.
    pub(crate) fn handle_notification_operation_outcome(
        &mut self,
        outcome: service_api::OperationOutcome,
    ) -> Task<Message> {
        let Some(state) = self.pending_action_plans.get_mut(&outcome.plan_id) else {
            // Outcome for an unknown plan - either the plan was already
            // completed (race), or this is a replay from a previous
            // incarnation whose plan never crossed into `pending_action_plans`
            // (UI restarted between dispatch and outcome). Drop quietly.
            log::debug!(
                "OperationOutcome for unknown plan {:?} (op {}) - dropping",
                outcome.plan_id,
                outcome.operation_id.0,
            );
            return Task::none();
        };
        if !state.applied_outcomes.insert(outcome.operation_id.0) {
            log::debug!(
                "OperationOutcome plan {:?} op {} already applied - dropping duplicate",
                outcome.plan_id,
                outcome.operation_id.0,
            );
            return Task::none();
        }
        let action_outcome = crate::action_wire::wire_outcome_to_action_outcome(outcome.result);
        state.outcomes.push((outcome.operation_id.0, action_outcome));
        Task::none()
    }

    /// Wire `ActionCompleted` notification arrival to
    /// `Message::ActionCompleted` so the existing post-completion
    /// pipeline (toast, undo eligibility, optimistic rollback on
    /// failure, auto-advance) runs against the assembled outcomes.
    pub(crate) fn handle_notification_action_completed(
        &mut self,
        completion: &service_api::ActionCompleted,
    ) -> Task<Message> {
        // Send jobs are quiet (no per-op OperationOutcome); the
        // ActionCompleted's `plan_id` field carries the UI-generated
        // `send_id`. Route to the compose window if we have one
        // tracking this id; the worker's PlanSummary tells us
        // success vs failure.
        if let Some(window_id) = self.in_flight_sends.remove(&completion.plan_id) {
            let outcome = if completion.summary.remote_succeeded > 0 {
                rtsk::actions::ActionOutcome::Success
            } else {
                rtsk::actions::ActionOutcome::Failed {
                    error: rtsk::actions::ActionError::remote(
                        "Send failed - see Service log for detail",
                    ),
                }
            };
            return Task::done(Message::SendCompleted { window_id, outcome });
        }
        let Some(state) = self.pending_action_plans.remove(&completion.plan_id) else {
            log::debug!(
                "ActionCompleted for unknown plan {:?} - dropping",
                completion.plan_id,
            );
            return Task::none();
        };
        let crate::app::PendingActionPlan {
            plan,
            mut outcomes,
            state: _,
            applied_outcomes: _,
            undo_description,
        } = state;
        // Release the per-target throttle entries so a follow-up action
        // against the same threads dispatches without the 200 ms wait.
        for (account_id, thread_id, _) in &plan.operations {
            self.action_throttle
                .remove(&(account_id.clone(), thread_id.clone()));
        }
        // Sort by operation_id so the outcomes Vec aligns with the
        // plan.operations order (op_id == index into operations).
        outcomes.sort_by_key(|(op_id, _)| *op_id);

        let expected = u32::try_from(plan.operations.len()).unwrap_or(u32::MAX);
        if outcomes.len() < plan.operations.len() {
            // Some OperationOutcome notifications were dropped or arrived
            // after ActionCompleted. Backfill the missing slots with a
            // synthetic LocalOnly so the existing handler doesn't index
            // off the end. Logged at warn so the gap is visible.
            log::warn!(
                "plan {:?} completed with only {} of {} outcomes",
                completion.plan_id,
                outcomes.len(),
                plan.operations.len(),
            );
            let present: std::collections::HashSet<u32> =
                outcomes.iter().map(|(id, _)| *id).collect();
            for op_id in 0..expected {
                if !present.contains(&op_id) {
                    outcomes.push((
                        op_id,
                        rtsk::actions::ActionOutcome::LocalOnly {
                            reason: rtsk::actions::ActionError::remote(
                                "OperationOutcome notification missing on the wire",
                            ),
                            retryable: false,
                        },
                    ));
                }
            }
            outcomes.sort_by_key(|(op_id, _)| *op_id);
        }
        let outcomes_vec: Vec<rtsk::actions::ActionOutcome> =
            outcomes.into_iter().map(|(_, o)| o).collect();
        // Phase 2 task 14: inverse plans dispatched by an undo route
        // through `Message::UndoCompleted` (toast + nav + thread-list
        // reload) so the pre-Phase-2 undo UX is preserved.
        if let Some(desc) = undo_description {
            Task::done(Message::UndoCompleted {
                desc,
                outcomes: outcomes_vec,
            })
        } else {
            Task::done(Message::ActionCompleted {
                plan,
                outcomes: outcomes_vec,
            })
        }
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

    /// Undo a previously-completed action by dispatching the inverse
    /// plan via the standard `action.execute_plan` IPC path.
    ///
    /// Phase 2 task 14: undo no longer runs in-process via
    /// `ActionContext`; instead each `MailUndoPayload` is converted to
    /// an `ActionExecutionPlan` whose operations are the per-payload
    /// inverses, and dispatched through the same `dispatch_plan` that
    /// regular actions use. The plan's `behavior.undo` is set to
    /// `Irreversible` so the inverse plan does not push another undo
    /// entry (no redo support today).
    ///
    /// `pending_operations` cancellation is still UI-side: when the
    /// original action's retryable failure has parked it in the retry
    /// queue, the inverse arriving Service-side would race the retry.
    /// Cancel before dispatch so the inverse wins. This is a write
    /// against `ReadDbState` (escape hatch); Phase 6's UI write-surface
    /// lockdown removes it.
    pub(crate) fn dispatch_undo(
        &mut self,
        entry: &cmdk::UndoEntry<crate::action_resolve::MailUndoPayload>,
    ) -> Task<Message> {
        use crate::action_resolve as ar;
        use rtsk::actions::MailOperation;

        // Cancel any pending-ops entries that would re-fire the
        // original action while the inverse runs. Best-effort: a
        // failed cancel just means a redundant retry, not a
        // correctness bug (the inverse and the retry would both write
        // to the same end-state and the worker's idempotent
        // application drops the duplicate outcome).
        let cancel_targets: Vec<(String, String, &'static str)> = entry
            .payloads
            .iter()
            .flat_map(undo_cancel_targets)
            .collect();
        if !cancel_targets.is_empty() {
            let db = self.db.read_db_state();
            tokio::spawn(async move {
                use rtsk::db::pending_ops::db_pending_ops_cancel_for_resource;
                for (account_id, resource_id, op_type) in cancel_targets {
                    let _ = db_pending_ops_cancel_for_resource(
                        &db,
                        account_id,
                        resource_id,
                        op_type.to_string(),
                    )
                    .await;
                }
            });
        }

        // Build operations from the payload list. Each payload yields
        // one MailOperation per thread (uniform inverse). The plan's
        // behavior is keyed off the first op's category for the toast
        // text; we override `undo` to `Irreversible` so no redo entry
        // gets pushed.
        let mut operations: Vec<(String, String, MailOperation)> = Vec::new();
        for payload in &entry.payloads {
            for (account_id, thread_id, op) in undo_payload_to_ops(payload) {
                operations.push((account_id, thread_id, op));
            }
        }
        if operations.is_empty() {
            log::debug!("dispatch_undo: empty inverse plan; nothing to do");
            return Task::none();
        }
        // Inverse plans don't push another undo entry (no redo). The
        // success_label and post-success effect don't matter for the
        // inverse plan because `undo_description` routes the
        // completion to `Message::UndoCompleted`, which has its own
        // toast + nav + thread-list reload logic.
        let mut behavior = ar::completion_behavior(&operations[0].2);
        behavior.undo = ar::UndoBehavior::Irreversible;

        let plan = ar::ActionExecutionPlan {
            operations,
            behavior,
            compensation: ar::CompensationContext::None,
            optimistic: Vec::new(),
        };
        self.dispatch_plan_with_undo(plan, Some(entry.description.clone()))
    }
}

// ── Undo helpers ────────────────────────────────────────────────────

/// Convert a `MailUndoPayload` to a list of `(account, thread, MailOperation)`
/// inverse operations. One `MailOperation` per `(payload variant, thread_id)`
/// pair.
fn undo_payload_to_ops(
    payload: &crate::action_resolve::MailUndoPayload,
) -> Vec<(String, String, rtsk::actions::MailOperation)> {
    use crate::action_resolve::MailUndoPayload;
    use rtsk::actions::MailOperation;
    match payload {
        MailUndoPayload::Archive { account_id, thread_ids } => {
            let inbox = TagId::from("INBOX");
            thread_ids
                .iter()
                .map(|tid| {
                    (
                        account_id.clone(),
                        tid.clone(),
                        MailOperation::AddLabel { label_id: inbox.clone() },
                    )
                })
                .collect()
        }
        MailUndoPayload::Trash { account_id, thread_ids, source } => thread_ids
            .iter()
            .map(|tid| {
                let op = if let Some(folder) = source {
                    MailOperation::MoveToFolder {
                        dest: folder.clone(),
                        source: None,
                    }
                } else {
                    MailOperation::AddLabel {
                        label_id: TagId::from("INBOX"),
                    }
                };
                (account_id.clone(), tid.clone(), op)
            })
            .collect(),
        MailUndoPayload::MoveToFolder { account_id, thread_ids, source } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::MoveToFolder {
                        dest: source.clone(),
                        source: None,
                    },
                )
            })
            .collect(),
        MailUndoPayload::SetSpam { account_id, thread_ids, was_spam } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::SetSpam { to: *was_spam },
                )
            })
            .collect(),
        MailUndoPayload::SetRead { account_id, thread_ids, was_read } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::SetRead { to: *was_read },
                )
            })
            .collect(),
        MailUndoPayload::SetStarred { account_id, thread_ids, was_starred } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::SetStarred { to: *was_starred },
                )
            })
            .collect(),
        MailUndoPayload::SetPinned { account_id, thread_ids, was_pinned } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::SetPinned { to: *was_pinned },
                )
            })
            .collect(),
        MailUndoPayload::SetMuted { account_id, thread_ids, was_muted } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::SetMuted { to: *was_muted },
                )
            })
            .collect(),
        MailUndoPayload::AddLabel { account_id, thread_ids, label_id } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::RemoveLabel { label_id: label_id.clone() },
                )
            })
            .collect(),
        MailUndoPayload::RemoveLabel { account_id, thread_ids, label_id } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::AddLabel { label_id: label_id.clone() },
                )
            })
            .collect(),
        MailUndoPayload::Snooze { account_id, thread_ids } => thread_ids
            .iter()
            .map(|tid| {
                (
                    account_id.clone(),
                    tid.clone(),
                    MailOperation::Unsnooze,
                )
            })
            .collect(),
    }
}

/// `(account_id, resource_id, operation_type)` triples whose
/// pending-ops entries should be cancelled before the inverse plan
/// dispatches. The original action's pending retry would otherwise
/// fight the inverse on the next drainer pass.
fn undo_cancel_targets(
    payload: &crate::action_resolve::MailUndoPayload,
) -> Vec<(String, String, &'static str)> {
    use crate::action_resolve::MailUndoPayload;
    let (account_id, thread_ids, op_type): (&String, &Vec<String>, &'static str) = match payload {
        MailUndoPayload::Archive { account_id, thread_ids } => (account_id, thread_ids, "archive"),
        MailUndoPayload::Trash { account_id, thread_ids, .. } => (account_id, thread_ids, "trash"),
        MailUndoPayload::MoveToFolder { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "moveToFolder")
        }
        MailUndoPayload::SetSpam { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "spam")
        }
        MailUndoPayload::SetRead { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "markRead")
        }
        MailUndoPayload::SetStarred { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "star")
        }
        MailUndoPayload::AddLabel { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "addLabel")
        }
        MailUndoPayload::RemoveLabel { account_id, thread_ids, .. } => {
            (account_id, thread_ids, "removeLabel")
        }
        // Pin/Mute/Snooze are local-only: they never enqueue into
        // pending_operations, so there is nothing to cancel.
        MailUndoPayload::SetPinned { .. }
        | MailUndoPayload::SetMuted { .. }
        | MailUndoPayload::Snooze { .. } => return Vec::new(),
    };
    thread_ids
        .iter()
        .map(|tid| (account_id.clone(), tid.clone(), op_type))
        .collect()
}

// ── Snooze resurface ─────────────────────────────────────────

// ── Snooze resurface ─────────────────────────────────────────

impl ReadyApp {
    /// Phase 2 task 17: snooze resurfacing runs Service-side. The UI's
    /// 60 s `SnoozeTick` fires a `pending_ops.kick` that wakes the
    /// action worker; the worker walks the snooze table and unsnoozes
    /// due threads via the relocated `snooze::unsnooze` action.
    ///
    /// The follow-up nav + thread-list reload runs after a 1.5 s
    /// delay so the worker's drain has time to commit. The 1.5 s is
    /// invisible to the user (the cadence is 60 s; a 1.5 s lag is
    /// well below perception). A future Phase 3 refinement could
    /// introduce a `nav.changed` notification that closes this
    /// window; for v1 the timer is enough.
    pub(crate) fn handle_snooze_tick(&self) -> Task<Message> {
        let Some(client) = self.service_client.as_ref().cloned() else {
            return Task::none();
        };
        let kick = Task::perform(
            async move {
                if let Err(error) = client
                    .send_notification(service_api::ClientNotification::PendingOpsKick)
                    .await
                {
                    log::debug!("snooze tick kick failed: {error}");
                }
            },
            |()| Message::Noop,
        );
        // Schedule the follow-up reload via a synthetic
        // SnoozeResurfaceComplete with `Ok(1)` (any non-zero count
        // triggers `load_navigation_and_threads`). 1.5 s gives the
        // worker time to drain.
        let reload = Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            },
            |()| Message::SnoozeResurfaceComplete(Ok(1)),
        );
        Task::batch([kick, reload])
    }

    /// After unsnoozing due threads, reload navigation (unread counts) and thread list.
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
#[allow(dead_code)] // Reserved for text-param command path; not wired in yet.
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
