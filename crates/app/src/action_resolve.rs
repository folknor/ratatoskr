//! Action resolution: Intent + UI context → resolved operation or toggle plan.
//!
//! This module is the single point where user intent is collapsed into
//! a concrete, unambiguous mail operation. Toggle directions, source
//! folders, and spam direction are resolved here - not scattered across
//! dispatch code.
//!
//! Toggle intents that require per-thread state are represented as
//! `ResolveOutcome::PerThreadToggle`, NOT as fake resolved operations.

use rtsk::actions::{ActionOutcome, FolderId, MailOperation, TagId};

// ── Intent ──────────────────────────────────────────────

/// Raw user intent. Carries inline parameters but no resolved context.
#[derive(Debug, Clone)]
pub enum MailActionIntent {
    Archive,
    Trash,
    PermanentDelete,
    ToggleSpam,
    ToggleStar,
    ToggleRead,
    TogglePin,
    ToggleMute,
    MoveToFolder { folder_id: FolderId },
    AddLabel { label_id: TagId },
    RemoveLabel { label_id: TagId },
    Snooze { until: i64 },
    Unsubscribe,
}

// ── UI Context ──────────────────────────────────────────

/// UI state captured at resolution time. This is the contract between
/// the UI layer and action resolution - everything the resolver needs
/// to collapse ambiguity.
#[derive(Debug, Clone)]
pub struct UiContext {
    /// The sidebar's current selection.
    pub selection: types::SidebarSelection,
}

// ── Compensation ────────────────────────────────────────

/// Undo/recovery metadata captured at resolution time.
/// Never sent to core - lives in app crate only.
///
/// Only carries data that is NOT already in the `MailOperation`.
/// MoveToFolder source is in the operation (needed for local mutation).
/// Labels, toggles, and snooze derive undo from the operation itself.
/// Only Trash source is truly compensation-only (trash_local doesn't use it).
#[derive(Debug, Clone)]
pub enum CompensationContext {
    /// No extra compensation data needed. Undo (if applicable) is derivable
    /// from the operation: reverse a label add/remove, invert a toggle, etc.
    None,
    /// Source folder for undo-trash. trash_local doesn't need this for
    /// execution - it adds the TRASH label regardless. But undo needs to
    /// know where the thread came from to move it back.
    SourceFolder(Option<FolderId>),
}

// ── Resolved Intent ─────────────────────────────────────

/// Fully resolved intent: a pure operation + compensation metadata.
#[derive(Debug, Clone)]
pub struct ResolvedIntent {
    /// What core executes. Pure semantics.
    pub operation: MailOperation,
    /// What undo needs beyond the operation.
    pub compensation: CompensationContext,
}

// ── Toggle identification ───────────────────────────────

/// Which boolean field a toggle operates on. Used by `build_execution_plan`
/// to read prior state, compute per-thread operations, and record typed
/// optimistic mutations.
#[derive(Debug, Clone, Copy)]
pub enum ToggleField {
    Star,
    Read,
    Pin,
    Mute,
}

// ── Resolve outcome ─────────────────────────────────────

/// Result of resolving a user intent. Three distinct states - no overloaded
/// `Option` or implicit valid combinations.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    /// Fully resolved - same operation for all targets.
    Resolved(ResolvedIntent),
    /// Toggle - requires per-thread state to resolve direction.
    PerThreadToggle {
        field: ToggleField,
        compensation: CompensationContext,
    },
    /// Fire-and-forget - no core operation (e.g., Unsubscribe).
    NoOp,
}

// ── Optimistic mutations ────────────────────────────────

/// What was flipped in the UI before execution. Typed per field.
///
/// Phase C builds `MailUndoPayload` from `MailOperation` +
/// `CompensationContext` + `OptimisticMutation` without re-reading UI state.
#[derive(Debug, Clone)]
pub enum OptimisticMutation {
    SetStarred {
        account_id: String,
        thread_id: String,
        previous: bool,
    },
    SetRead {
        account_id: String,
        thread_id: String,
        previous: bool,
    },
    SetPinned {
        account_id: String,
        thread_id: String,
        previous: bool,
    },
    SetMuted {
        account_id: String,
        thread_id: String,
        previous: bool,
    },
}

// ── Completion behavior ─────────────────────────────────

/// How the UI should handle an action's completion.
/// Derived from `MailOperation` via exhaustive match - compiler forces
/// a decision for every variant.
#[derive(Debug, Clone)]
pub struct CompletionBehavior {
    pub view_effect: ViewEffect,
    pub post_success: PostSuccessEffect,
    pub undo: UndoBehavior,
    pub success_label: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewEffect {
    Stays,
    LeavesCurrentView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostSuccessEffect {
    None,
    RefreshNav,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoBehavior {
    Irreversible,
    Reversible,
}

/// Derive completion behavior from a mail operation (exhaustive match).
/// Lives in the app crate because view effects, toast text, and undo
/// policy are UI concerns - core's MailOperation should not know about them.
///
/// All operations in a plan share the same behavior class even when
/// concrete values differ (e.g., SetStarred { to: true } and
/// SetStarred { to: false } produce the same behavior).
pub fn completion_behavior(op: &MailOperation) -> CompletionBehavior {
    match op {
        MailOperation::Archive => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Archived",
        },
        MailOperation::Trash => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Moved to Trash",
        },
        MailOperation::PermanentDelete => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Irreversible,
            success_label: "Permanently deleted",
        },
        MailOperation::SetSpam { .. } => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Spam status toggled",
        },
        MailOperation::SetStarred { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Star toggled",
        },
        MailOperation::SetRead { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::RefreshNav,
            undo: UndoBehavior::Reversible,
            success_label: "Read status toggled",
        },
        MailOperation::SetPinned { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Pin toggled",
        },
        MailOperation::SetMuted { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Mute toggled",
        },
        MailOperation::MoveToFolder { .. } => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Moved to folder",
        },
        MailOperation::AddLabel { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Label applied",
        },
        MailOperation::RemoveLabel { .. } => CompletionBehavior {
            view_effect: ViewEffect::Stays,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Label removed",
        },
        MailOperation::Snooze { .. } => CompletionBehavior {
            view_effect: ViewEffect::LeavesCurrentView,
            post_success: PostSuccessEffect::None,
            undo: UndoBehavior::Reversible,
            success_label: "Snoozed",
        },
    }
}

// ── Undo payload ────────────────────────────────────────

/// Mail-domain undo compensation data. Lives in app crate.
/// No `description()` method - description is set at push time on `UndoEntry` (C3).
#[derive(Debug, Clone)]
pub enum MailUndoPayload {
    Archive {
        account_id: String,
        thread_ids: Vec<String>,
    },
    Trash {
        account_id: String,
        thread_ids: Vec<String>,
        source: Option<FolderId>,
    },
    MoveToFolder {
        account_id: String,
        thread_ids: Vec<String>,
        source: FolderId,
    },
    SetSpam {
        account_id: String,
        thread_ids: Vec<String>,
        was_spam: bool,
    },
    SetStarred {
        account_id: String,
        thread_ids: Vec<String>,
        was_starred: bool,
    },
    SetRead {
        account_id: String,
        thread_ids: Vec<String>,
        was_read: bool,
    },
    SetPinned {
        account_id: String,
        thread_ids: Vec<String>,
        was_pinned: bool,
    },
    SetMuted {
        account_id: String,
        thread_ids: Vec<String>,
        was_muted: bool,
    },
    AddLabel {
        account_id: String,
        thread_ids: Vec<String>,
        label_id: TagId,
    },
    RemoveLabel {
        account_id: String,
        thread_ids: Vec<String>,
        label_id: TagId,
    },
    Snooze {
        account_id: String,
        thread_ids: Vec<String>,
    },
}

/// Compute a direction-aware undo description from undo payloads.
/// More informative than just success_label - e.g., "Starred" vs "Unstarred"
/// instead of generic "Star toggled".
pub fn undo_description(payloads: &[MailUndoPayload]) -> String {
    if payloads.is_empty() {
        return String::new();
    }
    match &payloads[0] {
        MailUndoPayload::Archive { thread_ids, .. } => {
            format!("Archived {} thread(s)", thread_ids.len())
        }
        MailUndoPayload::Trash { thread_ids, .. } => {
            format!("Trashed {} thread(s)", thread_ids.len())
        }
        MailUndoPayload::MoveToFolder { thread_ids, .. } => {
            format!("Moved {} thread(s)", thread_ids.len())
        }
        MailUndoPayload::SetSpam { was_spam, .. } => {
            if *was_spam {
                "Removed from spam".to_string()
            } else {
                "Marked as spam".to_string()
            }
        }
        MailUndoPayload::SetStarred { was_starred, .. } => {
            if *was_starred { "Unstarred" } else { "Starred" }.to_string()
        }
        MailUndoPayload::SetRead { was_read, .. } => if *was_read {
            "Marked as unread"
        } else {
            "Marked as read"
        }
        .to_string(),
        MailUndoPayload::SetPinned { was_pinned, .. } => {
            if *was_pinned { "Unpinned" } else { "Pinned" }.to_string()
        }
        MailUndoPayload::SetMuted { was_muted, .. } => {
            if *was_muted { "Unmuted" } else { "Muted" }.to_string()
        }
        MailUndoPayload::AddLabel { .. } => "Added label".to_string(),
        MailUndoPayload::RemoveLabel { .. } => "Removed label".to_string(),
        MailUndoPayload::Snooze { .. } => "Snoozed".to_string(),
    }
}

// ── Toast formatting ────────────────────────────────────

/// Generate user-facing toast text from completion behavior + outcomes.
/// All string policy centralized here - the completion handler delegates
/// entirely to this function.
pub fn format_outcome_toast(behavior: &CompletionBehavior, outcomes: &[ActionOutcome]) -> String {
    let total = outcomes.len();
    let succeeded = outcomes
        .iter()
        .filter(|o| o.is_success() || o.is_local_only() || o.is_noop())
        .count();
    let failed = outcomes.iter().filter(|o| o.is_failed()).count();
    let any_local_only = outcomes.iter().any(ActionOutcome::is_local_only);

    if failed == total {
        format!("\u{26A0} {} failed", behavior.success_label)
    } else if failed > 0 {
        format!(
            "\u{26A0} {} {succeeded} of {total} threads \u{2014} {failed} failed",
            behavior.success_label
        )
    } else if any_local_only {
        format!(
            "\u{26A0} {} locally \u{2014} sync may revert this",
            behavior.success_label
        )
    } else {
        behavior.success_label.to_string()
    }
}

// ── Undo payload construction ───────────────────────────

/// Build undo payloads from a completed plan + outcomes (C2: no UI re-read).
///
/// Returns empty vec for irreversible actions or all-failed outcomes.
/// For toggles: groups by (account_id, previous_value).
/// For non-toggles: groups by account_id.
/// C5: skips Trash undo when source is None.
/// C4: all payloads go in one UndoEntry (caller handles that).
pub fn build_undo_payloads(
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Vec<MailUndoPayload> {
    if matches!(plan.behavior.undo, UndoBehavior::Irreversible) {
        return Vec::new();
    }
    if outcomes.iter().all(ActionOutcome::is_failed) {
        return Vec::new();
    }

    if !plan.optimistic.is_empty() {
        // Toggle path: group by (account_id, previous)
        build_toggle_undo_payloads(plan, outcomes)
    } else {
        // Non-toggle path: group by account_id
        build_standard_undo_payloads(plan, outcomes)
    }
}

fn build_toggle_undo_payloads(
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Vec<MailUndoPayload> {
    use std::collections::BTreeMap;

    // BTreeMap for deterministic payload order (C8).
    let mut by_key: BTreeMap<(&str, bool), Vec<String>> = BTreeMap::new();
    for (mutation, outcome) in plan.optimistic.iter().zip(outcomes.iter()) {
        if !(outcome.is_success() || outcome.is_local_only()) {
            continue;
        }
        let (account_id, thread_id, previous) = match mutation {
            OptimisticMutation::SetStarred {
                account_id,
                thread_id,
                previous,
            }
            | OptimisticMutation::SetRead {
                account_id,
                thread_id,
                previous,
            }
            | OptimisticMutation::SetPinned {
                account_id,
                thread_id,
                previous,
            }
            | OptimisticMutation::SetMuted {
                account_id,
                thread_id,
                previous,
            } => (account_id.as_str(), thread_id.clone(), *previous),
        };
        by_key
            .entry((account_id, previous))
            .or_default()
            .push(thread_id);
    }

    // Determine which toggle type from the first optimistic mutation
    let first_mutation = &plan.optimistic[0];
    by_key
        .into_iter()
        .map(|((account_id, prev), thread_ids)| match first_mutation {
            OptimisticMutation::SetStarred { .. } => MailUndoPayload::SetStarred {
                account_id: account_id.to_string(),
                thread_ids,
                was_starred: prev,
            },
            OptimisticMutation::SetRead { .. } => MailUndoPayload::SetRead {
                account_id: account_id.to_string(),
                thread_ids,
                was_read: prev,
            },
            OptimisticMutation::SetPinned { .. } => MailUndoPayload::SetPinned {
                account_id: account_id.to_string(),
                thread_ids,
                was_pinned: prev,
            },
            OptimisticMutation::SetMuted { .. } => MailUndoPayload::SetMuted {
                account_id: account_id.to_string(),
                thread_ids,
                was_muted: prev,
            },
        })
        .collect()
}

fn build_standard_undo_payloads(
    plan: &ActionExecutionPlan,
    outcomes: &[ActionOutcome],
) -> Vec<MailUndoPayload> {
    use std::collections::BTreeMap;

    // BTreeMap for deterministic payload order (C8).
    let mut by_account: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for ((account_id, thread_id, _), outcome) in plan.operations.iter().zip(outcomes.iter()) {
        if !(outcome.is_success() || outcome.is_local_only()) {
            continue;
        }
        by_account
            .entry(account_id.as_str())
            .or_default()
            .push(thread_id.clone());
    }

    // Use first operation to determine action type
    let first_op = &plan.operations[0].2;
    by_account
        .into_iter()
        .filter_map(|(account_id, thread_ids)| {
            let account_id = account_id.to_string();
            match first_op {
                MailOperation::Archive => Some(MailUndoPayload::Archive {
                    account_id,
                    thread_ids,
                }),
                MailOperation::Trash => {
                    // C5: skip if source unknown
                    let source = match &plan.compensation {
                        CompensationContext::SourceFolder(s) => s.clone(),
                        CompensationContext::None => None,
                    };
                    if source.is_none() {
                        return None;
                    }
                    Some(MailUndoPayload::Trash {
                        account_id,
                        thread_ids,
                        source,
                    })
                }
                MailOperation::MoveToFolder { source, .. } => {
                    let source = source.clone()?;
                    Some(MailUndoPayload::MoveToFolder {
                        account_id,
                        thread_ids,
                        source,
                    })
                }
                MailOperation::SetSpam { to } => Some(MailUndoPayload::SetSpam {
                    account_id,
                    thread_ids,
                    was_spam: !to,
                }),
                MailOperation::AddLabel { label_id } => Some(MailUndoPayload::AddLabel {
                    account_id,
                    thread_ids,
                    label_id: label_id.clone(),
                }),
                MailOperation::RemoveLabel { label_id } => Some(MailUndoPayload::RemoveLabel {
                    account_id,
                    thread_ids,
                    label_id: label_id.clone(),
                }),
                MailOperation::Snooze { .. } => Some(MailUndoPayload::Snooze {
                    account_id,
                    thread_ids,
                }),
                // Toggles use the optimistic path, PermanentDelete is irreversible
                _ => None,
            }
        })
        .collect()
}

// ── Execution plan ──────────────────────────────────────

/// Everything needed to execute an action and handle its completion.
/// Built from a `ResolveOutcome` + selected threads.
#[derive(Debug, Clone)]
pub struct ActionExecutionPlan {
    /// Per-target operations - always a flat vec, even for uniform actions.
    /// `operations[i]` corresponds to `outcomes[i]` after execution.
    pub operations: Vec<(String, String, MailOperation)>,
    /// Completion behavior - set at construction, read by completion handler (C1).
    pub behavior: CompletionBehavior,
    /// Compensation context from resolution (Trash source folder).
    pub compensation: CompensationContext,
    /// Optimistic UI mutations applied (toggles only). Empty for non-toggles.
    pub optimistic: Vec<OptimisticMutation>,
}

// ── Resolution ──────────────────────────────────────────

/// Resolve a user intent into a concrete outcome using UI context.
///
/// This is the single place where:
/// - Spam direction is decided (ToggleSpam → SetSpam { to: true/false })
/// - Source folders are captured (Trash, MoveToFolder)
/// - Toggles are identified for per-thread planning
/// - Fire-and-forget intents are classified
pub fn resolve_intent(intent: MailActionIntent, ctx: &UiContext) -> ResolveOutcome {
    match intent {
        MailActionIntent::Archive => ResolveOutcome::Resolved(ResolvedIntent {
            operation: MailOperation::Archive,
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Trash => {
            let source = ctx.selection.source_folder_for_undo();
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::Trash,
                compensation: CompensationContext::SourceFolder(source),
            })
        }
        MailActionIntent::PermanentDelete => ResolveOutcome::Resolved(ResolvedIntent {
            operation: MailOperation::PermanentDelete,
            compensation: CompensationContext::None,
        }),
        MailActionIntent::ToggleSpam => {
            let is_in_spam = ctx.selection.is_spam();
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::SetSpam { to: !is_in_spam },
                compensation: CompensationContext::None,
            })
        }
        MailActionIntent::ToggleStar => ResolveOutcome::PerThreadToggle {
            field: ToggleField::Star,
            compensation: CompensationContext::None,
        },
        MailActionIntent::ToggleRead => ResolveOutcome::PerThreadToggle {
            field: ToggleField::Read,
            compensation: CompensationContext::None,
        },
        MailActionIntent::TogglePin => ResolveOutcome::PerThreadToggle {
            field: ToggleField::Pin,
            compensation: CompensationContext::None,
        },
        MailActionIntent::ToggleMute => ResolveOutcome::PerThreadToggle {
            field: ToggleField::Mute,
            compensation: CompensationContext::None,
        },
        MailActionIntent::MoveToFolder { folder_id } => {
            let source = ctx.selection.source_folder_for_undo();
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::MoveToFolder {
                    dest: folder_id,
                    source,
                },
                compensation: CompensationContext::None,
            })
        }
        MailActionIntent::AddLabel { label_id } => ResolveOutcome::Resolved(ResolvedIntent {
            operation: MailOperation::AddLabel { label_id },
            compensation: CompensationContext::None,
        }),
        MailActionIntent::RemoveLabel { label_id } => ResolveOutcome::Resolved(ResolvedIntent {
            operation: MailOperation::RemoveLabel { label_id },
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Snooze { until } => ResolveOutcome::Resolved(ResolvedIntent {
            operation: MailOperation::Snooze { until },
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Unsubscribe => ResolveOutcome::NoOp,
    }
}

// ── Per-target planning ─────────────────────────────────

/// Build an execution plan from a resolve outcome + selected threads.
///
/// For `Resolved`: every thread gets the same operation.
/// For `PerThreadToggle`: reads per-thread state, computes per-thread
/// operations, records typed optimistic mutations, then flips UI - in
/// that strict order per thread.
///
/// Returns `None` only for `NoOp` (fire-and-forget intents like Unsubscribe).
pub fn build_execution_plan(
    outcome: ResolveOutcome,
    threads: &[(String, String)],
    thread_list: &mut crate::ui::thread_list::ThreadList,
) -> Option<ActionExecutionPlan> {
    match outcome {
        ResolveOutcome::Resolved(resolved) => {
            let behavior = completion_behavior(&resolved.operation);
            let operations = threads
                .iter()
                .map(|(a, t)| (a.clone(), t.clone(), resolved.operation.clone()))
                .collect();
            Some(ActionExecutionPlan {
                operations,
                behavior,
                compensation: resolved.compensation,
                optimistic: vec![],
            })
        }
        ResolveOutcome::PerThreadToggle {
            field,
            compensation,
        } => {
            // C1: to value doesn't affect behavior class
            let behavior = completion_behavior(&toggle_to_operation(field, true));
            let mut operations = Vec::with_capacity(threads.len());
            let mut optimistic = Vec::with_capacity(threads.len());

            for (account_id, thread_id) in threads {
                let thread = thread_list
                    .threads
                    .iter_mut()
                    .find(|t| t.account_id == *account_id && t.id == *thread_id);

                if let Some(t) = thread {
                    // I2: strict ordering - read prior, compute op, record mutation, flip UI
                    let previous = read_toggle_field(field, t);
                    let new_value = !previous;
                    operations.push((
                        account_id.clone(),
                        thread_id.clone(),
                        toggle_to_operation(field, new_value),
                    ));
                    optimistic.push(toggle_to_mutation(
                        field,
                        account_id.clone(),
                        thread_id.clone(),
                        previous,
                    ));
                    write_toggle_field(field, t, new_value);
                } else {
                    // Thread not in list (concurrent removal). Skip entirely -
                    // don't fabricate an operation or mutation. operations and
                    // optimistic stay aligned (both skip this thread).
                    log::debug!(
                        "build_execution_plan: toggle target {account_id}/{thread_id} not found in thread list, skipping"
                    );
                }
            }

            Some(ActionExecutionPlan {
                operations,
                behavior,
                compensation,
                optimistic,
            })
        }
        ResolveOutcome::NoOp => None,
    }
}

// ── Toggle helpers ──────────────────────────────────────

fn read_toggle_field(field: ToggleField, thread: &crate::db::Thread) -> bool {
    match field {
        ToggleField::Star => thread.is_starred,
        ToggleField::Read => thread.is_read,
        ToggleField::Pin => thread.is_pinned,
        ToggleField::Mute => thread.is_muted,
    }
}

fn write_toggle_field(field: ToggleField, thread: &mut crate::db::Thread, value: bool) {
    match field {
        ToggleField::Star => thread.is_starred = value,
        ToggleField::Read => thread.is_read = value,
        ToggleField::Pin => thread.is_pinned = value,
        ToggleField::Mute => thread.is_muted = value,
    }
}

fn toggle_to_operation(field: ToggleField, to: bool) -> MailOperation {
    match field {
        ToggleField::Star => MailOperation::SetStarred { to },
        ToggleField::Read => MailOperation::SetRead { to },
        ToggleField::Pin => MailOperation::SetPinned { to },
        ToggleField::Mute => MailOperation::SetMuted { to },
    }
}

fn toggle_to_mutation(
    field: ToggleField,
    account_id: String,
    thread_id: String,
    previous: bool,
) -> OptimisticMutation {
    match field {
        ToggleField::Star => OptimisticMutation::SetStarred {
            account_id,
            thread_id,
            previous,
        },
        ToggleField::Read => OptimisticMutation::SetRead {
            account_id,
            thread_id,
            previous,
        },
        ToggleField::Pin => OptimisticMutation::SetPinned {
            account_id,
            thread_id,
            previous,
        },
        ToggleField::Mute => OptimisticMutation::SetMuted {
            account_id,
            thread_id,
            previous,
        },
    }
}
