use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;

/// Dependencies needed by the action service.
///
/// Constructed once at app startup from pre-initialized stores.
/// All fields are cheaply cloneable (`Arc<Mutex<…>>` internally).
/// The service constructs `ProviderCtx` from these fields per-call —
/// callers never see `ProviderCtx`.
#[derive(Clone)]
pub struct ActionContext {
    pub db: DbState,
    pub body_store: BodyStoreState,
    pub inline_images: InlineImageStoreState,
    pub search: SearchState,
    pub encryption_key: [u8; 32],
    /// When true, `enqueue_if_retryable` is suppressed. Set by the
    /// pending-ops worker to prevent retried actions from re-enqueuing
    /// themselves (which would create duplicate pending ops).
    pub suppress_pending_enqueue: bool,
    /// Tracks threads with in-flight mutations. Key: `"{account_id}:{thread_id}"`.
    ///
    /// Policy: one mutation at a time per thread, regardless of action type.
    /// batch_execute checks+inserts before dispatch, removes after.
    /// process_pending_ops skips in-flight threads without incrementing retry.
    pub in_flight: Arc<Mutex<HashSet<String>>>,
}
