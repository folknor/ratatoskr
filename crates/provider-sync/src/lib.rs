//! Support helpers used by the Bifrost change-stream consumer and retained
//! provider-specific auxiliary sync paths.

pub mod consumer_support;
pub mod gmail;
pub mod graph;
pub mod imap;
pub mod jmap;
mod keyword_membership;
pub(crate) mod persistence;
mod seen_ingest;
mod thread_membership;
