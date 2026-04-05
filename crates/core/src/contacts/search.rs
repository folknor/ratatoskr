//! Contact search — re-export shim.
//!
//! Implementation lives in `db::queries_extra::contact_search`.
//! This module preserves the `rtsk::contacts::search::*` import path.

pub use db::db::queries_extra::contact_search::*;
