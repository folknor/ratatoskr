//! Service-side account CRUD helpers shared across IPC entry points.
//!
//! `create_account_inner` is the single creation path: `account.create`
//! (Phase 6a) and `oauth.exchange_code` (Phase 6b) both go through
//! it. Future post-create side effects (default folder set, initial
//! sync trigger, etc.) hook in here instead of being duplicated at
//! each entry point.

pub(crate) mod create;

pub(crate) use create::create_account_inner;
