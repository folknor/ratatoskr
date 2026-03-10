pub mod commands;
pub(crate) mod config;
mod convert;
mod folder_mapper;
pub(crate) mod imap_delta;
pub(crate) mod imap_initial;
mod pipeline;
pub(crate) mod types;

use std::collections::HashSet;
use std::sync::Mutex;

/// Per-account sync lock to prevent concurrent syncs.
pub struct SyncState {
    active_syncs: Mutex<HashSet<String>>,
}

impl SyncState {
    pub fn new() -> Self {
        Self {
            active_syncs: Mutex::new(HashSet::new()),
        }
    }

    /// Try to acquire a sync lock for the given account.
    /// Returns `true` if the lock was acquired, `false` if already syncing.
    pub fn try_lock_account(&self, account_id: &str) -> bool {
        let mut set = self.active_syncs.lock().expect("sync lock poisoned");
        set.insert(account_id.to_string())
    }

    /// Release the sync lock for the given account.
    pub fn unlock_account(&self, account_id: &str) {
        let mut set = self.active_syncs.lock().expect("sync lock poisoned");
        set.remove(account_id);
    }
}
