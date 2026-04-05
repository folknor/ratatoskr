//! Contact search — re-export shim.
//!
//! Implementation lives in `db::queries_extra::contact_search`.
//! This module preserves the `rtsk::contacts::search::*` import path.

pub use crate::db::queries_extra::contact_search::*;
