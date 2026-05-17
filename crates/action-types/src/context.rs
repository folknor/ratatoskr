use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use store::body_store::BodyStoreReadState;
use db::db::ReadDbState;
use search::SearchReadState;
use store::inline_image_store::InlineImageStoreReadState;

/// Dependencies needed by the action service.
///
/// Constructed once at app startup from pre-initialized stores.
/// All fields are cheaply cloneable (`Arc<Mutex<…>>` internally).
/// The service constructs `ProviderCtx` from these fields per-call -
/// callers never see `ProviderCtx`.
///
/// `db` is the read-half view used by mail action paths; `write_db`
/// is the write-half view used by the calendar dispatch path
/// (`run_one_calendar` builds `CalendarActionContext { write_db,
/// ... }` from this field). Both wrap the same connection arc; the
/// distinction is type-level - holding `write_db` keeps the
/// "writes go through the writer half" invariant compile-checked
/// without the legacy raw-Arc writer-state end-run the worker used to
/// perform per calendar op.
#[derive(Clone)]
pub struct ActionContext {
    pub db: ReadDbState,
    pub write_db: service_state::WriteDbState,
    pub body_store: BodyStoreReadState,
    pub inline_images: InlineImageStoreReadState,
    pub search: SearchReadState,
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
        let aid = account_id.to_string();
        let tid = thread_id.to_string();
        let exists = self
            .db
            .with_conn_sync(move |conn| {
                db::db::queries_extra::action_helpers::thread_exists_sync(conn, &aid, &tid)
            })
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

/// Phase 6c sibling of `ActionContext` for the calendar action
/// pipeline. Sized to what `cal::actions::*` actually needs: the DB
/// writer-half + the encryption key. Email-shaped fields
/// (`body_store`, `inline_images`, `search`, `in_flight`,
/// `suppress_pending_enqueue`) have no analogue in calendar
/// dispatch and would be dead weight here.
///
/// Constructible only by code that already has access to
/// `WriteDbState`. Phase 6b's Cargo-level lockdown forbids the app
/// crate from depending on `service-state` directly; Phase 6c-11
/// extends that to the transitive graph after `app -> cal` is
/// removed in 6c-10. Together, those rules constrain
/// `CalendarActionContext` construction to crates inside the
/// Service's transitive cone (today: `service`).
///
/// Field visibility is `pub` on purpose: a `pub(crate)` constructor
/// with `pub(crate)` fields would still let any crate inside
/// `action-types` build one, while blocking `service` (the actual
/// caller). The Cargo-graph lockdown does the gating; the type's
/// shape is just data.
#[derive(Clone)]
pub struct CalendarActionContext {
    pub write_db: service_state::WriteDbState,
    pub read_db: ReadDbState,
    pub encryption_key: [u8; 32],
}
