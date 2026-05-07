//! Phase 7-4: relocated single global sweep lock for the attachment
//! cache.
//!
//! Pre-Phase-7 this lived as a private `static` inside
//! `handlers/attachment.rs`. Phase 7-4 moves it into a shared module
//! so the new `ExtractRuntime` worker can also acquire the read guard
//! when reading bytes from `attachment_cache/<hash>`. Both
//! `attachment.fetch` and `extract.rs` import `SWEEP_LOCK` from here;
//! eviction (`attachment.eviction_kick`) is the only writer.
//!
//! Semantics (unchanged from the pre-Phase-7 contract):
//!
//! - **Read guard**: held during a cache read or a cache-miss
//!   commit-then-rename. Multiple readers run in parallel.
//! - **Write guard**: held during an eviction sweep. Blocks readers
//!   for the duration of the sweep AND is blocked by in-flight reads.
//! - **Per-worker acquisition**: each call site acquires the lock at
//!   its operation boundary, not for the lifetime of any handle.
//!   Eviction can run between two unrelated calls; only an in-flight
//!   call holds the read guard.

use tokio::sync::RwLock;

/// Single global sweep lock. `RwLock` so concurrent fetches on the
/// hit and miss paths (read guards) run in parallel while the
/// eviction sweep (write guard) blocks them and is blocked by
/// in-flight fetches. A slow sweep on one tick is not re-entered
/// when the next tick lands within `NOTIFY_CAP=4` queued kicks; the
/// second kick acquires the write lock once the first finishes
/// (back-to-back sweeps just see the cache already under cap and
/// return immediately).
pub(crate) static SWEEP_LOCK: RwLock<()> = RwLock::const_new(());
