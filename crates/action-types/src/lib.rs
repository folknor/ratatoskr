//! Shared type contract for the action service.
//!
//! Phase 5 prerequisite: extracted out of `service::actions` so that `cal`
//! and any other crate that needs to talk in action-pipeline types
//! (`ActionContext`, `ActionError`, `ActionOutcome`, `MutationLog`) can
//! depend on this crate directly instead of going through the
//! `rtsk::actions` shim. The shim's existence forces a `rtsk -> service`
//! edge that prevents `service -> cal` and `service -> rtsk` (needed by
//! `CalendarRuntime` and the GAL handler respectively).
//!
//! `service::actions` continues to re-export everything from this crate
//! so service-internal call sites remain unchanged.

mod context;
mod log;
mod outcome;

pub use context::{ActionContext, FlightGuard};
pub use log::MutationLog;
pub use outcome::{ActionError, ActionOutcome, RemoteFailureKind};
