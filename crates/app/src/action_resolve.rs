//! Action resolution: Intent + UI context → resolved operation.
//!
//! This module is the single point where user intent is collapsed into
//! a concrete, unambiguous mail operation. Toggle directions, source
//! folders, and spam direction are resolved here — not scattered across
//! dispatch code.

use ratatoskr_core::actions::{FolderId, MailOperation, TagId};

// ── Intent ──────────────────────────────────────────────

/// Raw user intent. Carries inline parameters but no resolved context.
/// Replaces `EmailAction` (will be unified in Phase B/C).
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
/// Labels, toggles, and snooze derive their undo from the operation itself.
/// Only folder-provenance (where a thread came from) is truly external.
#[derive(Debug, Clone)]
pub enum CompensationContext {
    /// No extra compensation data needed. Undo (if applicable) is derivable
    /// from the operation: reverse a label add/remove, invert a toggle, etc.
    None,
    /// Source folder for undo-trash / undo-move. This is the one piece of
    /// data that cannot be derived from the operation — it comes from the
    /// UI context at resolution time.
    SourceFolder(Option<FolderId>),
}

// ── Resolved Intent ─────────────────────────────────────

/// Fully resolved intent: a pure operation + compensation metadata.
/// Produced by `resolve_intent()`, consumed by dispatch.
#[derive(Debug, Clone)]
pub struct ResolvedIntent {
    /// What core executes. Pure semantics, no UI metadata.
    pub operation: MailOperation,
    /// What undo needs. Not sent to core.
    pub compensation: CompensationContext,
}

// ── Resolution ──────────────────────────────────────────

/// Resolve a user intent into a concrete operation using UI context.
///
/// This is the single place where:
/// - Toggle directions are decided (ToggleSpam → SetSpam { to: true/false })
/// - Source folders are captured (Trash, MoveToFolder)
/// - All ambiguity is collapsed
///
/// Returns `None` for intents that don't produce a core operation
/// (e.g., Unsubscribe is fire-and-forget with no backend action).
pub fn resolve_intent(intent: MailActionIntent, ctx: &UiContext) -> Option<ResolvedIntent> {
    match intent {
        MailActionIntent::Archive => Some(ResolvedIntent {
            operation: MailOperation::Archive,
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Trash => {
            let source = ctx.selected_label.clone().map(FolderId::from);
            Some(ResolvedIntent {
                operation: MailOperation::Trash,
                compensation: CompensationContext::SourceFolder(source),
            })
        }
        MailActionIntent::PermanentDelete => Some(ResolvedIntent {
            operation: MailOperation::PermanentDelete,
            compensation: CompensationContext::None,
        }),
        MailActionIntent::ToggleSpam => {
            let is_in_spam = ctx.selected_label.as_deref() == Some("SPAM");
            Some(ResolvedIntent {
                operation: MailOperation::SetSpam { to: !is_in_spam },
                compensation: CompensationContext::None,
            })
        }
        // Toggle intents cannot be resolved here — they require per-thread
        // state (current starred/read/pinned/muted value) which is only available
        // during per-target planning (Phase B). Callers must handle these before
        // calling resolve_intent.
        MailActionIntent::ToggleStar
        | MailActionIntent::ToggleRead
        | MailActionIntent::TogglePin
        | MailActionIntent::ToggleMute => None,
        MailActionIntent::MoveToFolder { folder_id } => {
            let source = ctx.selected_label.clone().map(FolderId::from);
            Some(ResolvedIntent {
                operation: MailOperation::MoveToFolder { dest: folder_id },
                compensation: CompensationContext::SourceFolder(source),
            })
        }
        MailActionIntent::AddLabel { label_id } => Some(ResolvedIntent {
            operation: MailOperation::AddLabel { label_id },
            // Undo derives the label from the operation (reverse = remove same label).
            compensation: CompensationContext::None,
        }),
        MailActionIntent::RemoveLabel { label_id } => Some(ResolvedIntent {
            operation: MailOperation::RemoveLabel { label_id },
            // Undo derives the label from the operation (reverse = add same label).
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Snooze { until } => Some(ResolvedIntent {
            operation: MailOperation::Snooze { until },
            compensation: CompensationContext::None,
        }),
        MailActionIntent::Unsubscribe => None, // fire-and-forget, no core operation
    }
}
