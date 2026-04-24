use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;
use store::inline_image_store::InlineImageStoreState;

/// Dependencies needed by the action service.
///
/// Constructed once at app startup from pre-initialized stores.
/// All fields are cheaply cloneable (`Arc<Mutex<…>>` internally).
/// The service constructs `ProviderCtx` from these fields per-call -
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
    /// Use `try_acquire_flight` to check+insert atomically; the returned
    /// `FlightGuard` removes the key on drop.
    pub in_flight: Arc<Mutex<HashSet<String>>>,
}

impl ActionContext {
    /// Try to acquire the in-flight guard for a thread. Returns `Some(FlightGuard)`
    /// if the thread was not already in flight (guard inserted). Returns `None` if
    /// the thread is already in flight. The guard removes the key on drop.
    pub fn try_acquire_flight(&self, account_id: &str, thread_id: &str) -> Option<FlightGuard> {
        let key = format!("{account_id}:{thread_id}");
        let mut set = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if set.insert(key.clone()) {
            Some(FlightGuard {
                set: Arc::clone(&self.in_flight),
                key,
            })
        } else {
            None
        }
    }

    /// Verify that a thread exists in the database. Returns `Ok(())` if it does,
    /// or `Err(ActionError::NotFound)` if it doesn't.
    pub fn verify_thread_exists(
        &self,
        account_id: &str,
        thread_id: &str,
    ) -> Result<(), super::outcome::ActionError> {
        let conn = self.db.conn();
        let conn = conn
            .lock()
            .map_err(|e| super::outcome::ActionError::db(format!("db lock: {e}")))?;
        let exists = crate::db::queries_extra::action_helpers::thread_exists_sync(
            &conn,
            account_id,
            thread_id,
        )
        .map_err(super::outcome::ActionError::db)?;
        if exists {
            Ok(())
        } else {
            Err(super::outcome::ActionError::not_found(format!(
                "thread {thread_id} not found for account {account_id}"
            )))
        }
    }

    /// Check if a thread is currently in flight (read-only, no insertion).
    pub fn is_in_flight(&self, account_id: &str, thread_id: &str) -> bool {
        let key = format!("{account_id}:{thread_id}");
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key)
    }
}

/// RAII guard that removes the in-flight key on drop.
/// Guarantees cleanup even on early returns or panics.
pub struct FlightGuard {
    set: Arc<Mutex<HashSet<String>>>,
    key: String,
}

impl Drop for FlightGuard {
    fn drop(&mut self) {
        let mut set = self.set.lock().unwrap_or_else(PoisonError::into_inner);
        set.remove(&self.key);
    }
}
