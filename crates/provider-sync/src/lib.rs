//! Service-side sync trait + per-provider impls.
//!
//! Phase 6d-B carves the sync-trait surface out of `common`.
//! `common::ProviderOps` keeps only the action / send / draft / folder
//! / profile / connection methods - none of which take a writer-half
//! handle - so `common` no longer needs to depend on `service-state`.
//! The sync methods (`sync_initial`, `sync_delta`) and their
//! `SyncProviderCtx` parameter relocate here, where the
//! `service-state` dep is legitimate (Service-side).
//!
//! `ProviderSyncOps` inherits `ProviderOps` so any `&dyn ProviderSyncOps`
//! is also usable as a `&dyn ProviderOps` via supertrait method
//! resolution. The provider registry returns `Box<dyn ProviderSyncOps>`
//! and the action-side dispatch keeps calling action methods directly
//! on it without an explicit upcast.
//!
//! The orphan-impls (`impl ProviderSyncOps for {Gmail,Graph,Imap}Ops`)
//! live in the per-provider sub-modules. The orphan rule
//! is satisfied because the trait is local to this crate; the impl
//! targets are foreign types from the provider crates.

pub mod consumer_support;
pub mod gmail;
mod gmail_impl;
pub mod graph;
mod graph_impl;
pub mod imap;
mod imap_impl;
pub mod jmap;
mod keyword_membership;
pub(crate) mod persistence;
mod seen_ingest;
mod thread_membership;

use async_trait::async_trait;
use common::error::ProviderError;
use common::ops::ProviderOps;
use common::types::SyncResult;
use db::db::ReadDbState;
use db::progress::ProgressReporter;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use tokio_util::sync::CancellationToken;

/// Sync-side context for provider sync methods (`sync_initial`,
/// `sync_delta`).
///
/// Phase 6d-B relocated this from `common::types`. The shape is
/// unchanged: writer-half handles for the four content stores plus a
/// progress reporter and a cancellation token. Carrying the token on
/// the ctx lets every leaf call observe cancellation without an extra
/// parameter (Phase 3 task 6).
pub struct SyncProviderCtx<'a> {
    pub account_id: &'a str,
    pub db: &'a WriteDbState,
    pub read_db: &'a ReadDbState,
    pub body_store: &'a BodyStoreWriteState,
    pub inline_images: &'a InlineImageStoreWriteState,
    pub search: &'a SearchWriteHandle,
    pub progress: &'a dyn ProgressReporter,
    pub cancellation_token: &'a CancellationToken,
}

/// Sync-side trait for provider implementations.
///
/// Inherits `ProviderOps` so a `&dyn ProviderSyncOps` is usable wherever
/// a `&dyn ProviderOps` is needed (via supertrait method resolution -
/// the action dispatch path keeps calling `provider.archive(...)` etc.
/// directly on it). The provider registry post-6d-B returns
/// `Box<dyn ProviderSyncOps>` so a single trait object covers both the
/// action and sync surfaces.
#[async_trait]
pub trait ProviderSyncOps: ProviderOps {
    async fn sync_initial(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError>;
    async fn sync_delta(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError>;
}
