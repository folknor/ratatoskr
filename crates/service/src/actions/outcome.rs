//! Re-export shim: the canonical definitions live in the `action-types`
//! crate. See `docs/service/phase-5-plan.md` § "Prerequisite".

pub use action_types::{ActionError, ActionOutcome, RemoteFailureKind};
