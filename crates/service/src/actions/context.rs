//! Re-export shim: the canonical definition of `ActionContext` lives in
//! the `action-types` crate so `cal::actions` can import it without a
//! dependency cycle. See `docs/service/phase-5-plan.md` § "Prerequisite".
//!
//! Sibling modules in `service::actions` continue to do `use super::context::ActionContext`;
//! that path still resolves through this shim.

pub use action_types::ActionContext;
