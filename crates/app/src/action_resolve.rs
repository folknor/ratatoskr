//! Action resolution: Intent + UI context → resolved operation or toggle plan.
//!
//! This module is the single point where user intent is collapsed into
//! a concrete, unambiguous mail operation. Toggle directions, source
//! folders, and spam direction are resolved here — not scattered across
//! dispatch code.
//!
//! Toggle intents that require per-thread state are represented as
//! `ResolveOutcome::PerThreadToggle`, NOT as fake resolved operations.

use ratatoskr_core::actions::{FolderId, MailOperation, TagId};

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
/// the UI layer and action resolution — everything the resolver needs
/// to collapse ambiguity.
#[derive(Debug, Clone)]
pub struct UiContext {
    /// The sidebar's active label/folder (e.g., "INBOX", "SPAM", a folder ID).
    pub selected_label: Option<String>,
}

// ── Compensation ────────────────────────────────────────

/// Undo/recovery metadata captured at resolution time.
/// Never sent to core — lives in app crate only.
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
    /// execution — it adds the TRASH label regardless. But undo needs to
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

/// Result of resolving a user intent. Three distinct states — no overloaded
/// `Option` or implicit valid combinations.
#[derive(Debug, Clone)]
pub enum ResolveOutcome {
    /// Fully resolved — same operation for all targets.
    Resolved(ResolvedIntent),
    /// Toggle — requires per-thread state to resolve direction.
    PerThreadToggle {
        field: ToggleField,
        compensation: CompensationContext,
    },
    /// Fire-and-forget — no core operation (e.g., Unsubscribe).
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

// ── Execution plan ──────────────────────────────────────

/// Everything needed to execute an action and handle its completion.
/// Built from a `ResolveOutcome` + selected threads.
pub struct ActionExecutionPlan {
    /// Per-target operations — always a flat vec, even for uniform actions.
    /// `operations[i]` corresponds to `outcomes[i]` after execution.
    pub operations: Vec<(String, String, MailOperation)>,
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
            let source = ctx.selected_label.clone().map(FolderId::from);
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
            let is_in_spam = ctx.selected_label.as_deref() == Some("SPAM");
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
            let source = ctx.selected_label.clone().map(FolderId::from);
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::MoveToFolder {
                    dest: folder_id,
                    source,
                },
                compensation: CompensationContext::None,
            })
        }
        MailActionIntent::AddLabel { label_id } => {
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::AddLabel { label_id },
                compensation: CompensationContext::None,
            })
        }
        MailActionIntent::RemoveLabel { label_id } => {
            ResolveOutcome::Resolved(ResolvedIntent {
                operation: MailOperation::RemoveLabel { label_id },
                compensation: CompensationContext::None,
            })
        }
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
/// operations, records typed optimistic mutations, then flips UI — in
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
            let operations = threads
                .iter()
                .map(|(a, t)| (a.clone(), t.clone(), resolved.operation.clone()))
                .collect();
            Some(ActionExecutionPlan {
                operations,
                compensation: resolved.compensation,
                optimistic: vec![],
            })
        }
        ResolveOutcome::PerThreadToggle {
            field,
            compensation,
        } => {
            let mut operations = Vec::with_capacity(threads.len());
            let mut optimistic = Vec::with_capacity(threads.len());

            for (account_id, thread_id) in threads {
                let thread = thread_list
                    .threads
                    .iter_mut()
                    .find(|t| t.account_id == *account_id && t.id == *thread_id);

                if let Some(t) = thread {
                    // I2: strict ordering — read prior, compute op, record mutation, flip UI
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
                    // Thread not in list (concurrent removal). Skip entirely —
                    // don't fabricate an operation or mutation. operations and
                    // optimistic stay aligned (both skip this thread).
                    log::debug!(
                        "build_execution_plan: toggle target {account_id}/{thread_id} not found in thread list, skipping"
                    );
                }
            }

            Some(ActionExecutionPlan {
                operations,
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
