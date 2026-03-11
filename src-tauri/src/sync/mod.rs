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
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

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

struct SyncQueueInner {
    active: bool,
    pending_account_ids: Vec<String>,
    waiters: Vec<oneshot::Sender<()>>,
}

/// Serialized sync queue for manual/frontend-triggered sync runs.
pub struct SyncQueueState {
    inner: Mutex<SyncQueueInner>,
}

impl SyncQueueState {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SyncQueueInner {
                active: false,
                pending_account_ids: Vec::new(),
                waiters: Vec::new(),
            }),
        }
    }

    /// Queue account IDs and return a receiver that resolves when the queue
    /// drains completely, matching the old TS-side `runSync()` contract.
    pub fn enqueue(&self, account_ids: &[String]) -> (bool, oneshot::Receiver<()>) {
        let mut inner = self.inner.lock().expect("sync queue lock poisoned");
        for account_id in account_ids {
            if !inner.pending_account_ids.contains(account_id) {
                inner.pending_account_ids.push(account_id.clone());
            }
        }

        let (tx, rx) = oneshot::channel();
        inner.waiters.push(tx);

        let should_spawn = !inner.active;
        if should_spawn {
            inner.active = true;
        }

        (should_spawn, rx)
    }

    pub fn take_pending_batch(&self) -> Vec<String> {
        let mut inner = self.inner.lock().expect("sync queue lock poisoned");
        std::mem::take(&mut inner.pending_account_ids)
    }

    pub fn finish_if_idle(&self) -> Option<Vec<oneshot::Sender<()>>> {
        let mut inner = self.inner.lock().expect("sync queue lock poisoned");
        if inner.pending_account_ids.is_empty() {
            inner.active = false;
            return Some(std::mem::take(&mut inner.waiters));
        }
        None
    }
}

/// Background periodic sync task controller.
pub struct BackgroundSyncState {
    task: Mutex<Option<JoinHandle<()>>>,
}

impl BackgroundSyncState {
    pub fn new() -> Self {
        Self {
            task: Mutex::new(None),
        }
    }

    pub fn replace(&self, task: JoinHandle<()>) {
        let mut current = self.task.lock().expect("background sync lock poisoned");
        if let Some(existing) = current.take() {
            existing.abort();
        }
        *current = Some(task);
    }

    pub fn stop(&self) {
        let mut current = self.task.lock().expect("background sync lock poisoned");
        if let Some(task) = current.take() {
            task.abort();
        }
    }
}
