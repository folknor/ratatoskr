//! Re-exports of timezone-aware datetime helpers from the `db` crate.
//!
//! The implementation lives in `db::db::time` because `db` is the lowest
//! crate in the dep graph that needs DST-aware resolution (RRULE expansion
//! at query time). `common` already depends on `db`, so re-exporting here
//! keeps the `common::time::resolve_local_to_timestamp` import path stable
//! for everything downstream.

pub use db::db::time::resolve_local_to_timestamp;
