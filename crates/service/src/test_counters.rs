//! Test-only service counters and write crash injection.
//!
//! Harness M3 needs deterministic service-side observations that do not
//! depend on wall-clock sleeps: read a counter before/after an action, and
//! arm a crash after the Nth write in a named class.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

const ACTION_BATCH_EXECUTE: &str = "action.batch_execute";
const ACTION_JOURNAL_WRITE: &str = "action.journal_write";
const SEARCH_WRITE: &str = "search.write";
const SEARCH_INDEX: &str = "search.index";
const SEARCH_DELETE: &str = "search.delete";
const SEARCH_CLEAR: &str = "search.clear";
const SEARCH_FLUSH: &str = "search.flush";

static ACTION_BATCH_EXECUTE_COUNT: AtomicU64 = AtomicU64::new(0);
static ACTION_JOURNAL_WRITE_COUNT: AtomicU64 = AtomicU64::new(0);
static SEARCH_WRITE_COUNT: AtomicU64 = AtomicU64::new(0);
static SEARCH_INDEX_COUNT: AtomicU64 = AtomicU64::new(0);
static SEARCH_DELETE_COUNT: AtomicU64 = AtomicU64::new(0);
static SEARCH_CLEAR_COUNT: AtomicU64 = AtomicU64::new(0);
static SEARCH_FLUSH_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
struct CrashRule {
    kind: String,
    trigger_at: u64,
}

static CRASH_RULES: OnceLock<Mutex<Vec<CrashRule>>> = OnceLock::new();

pub(crate) fn read(kind: &str) -> Option<u64> {
    counter(kind).map(|counter| counter.load(Ordering::SeqCst))
}

pub(crate) fn record(kind: &str) -> u64 {
    let Some(counter) = counter(kind) else {
        log::warn!("test counter record for unknown kind {kind:?}");
        return 0;
    };
    let value = counter.fetch_add(1, Ordering::SeqCst) + 1;
    maybe_crash(kind, value);
    value
}

pub(crate) fn configure_crash(kind: String, n: u64) -> Result<(), String> {
    if n == 0 {
        return Err("n must be greater than zero".into());
    }
    let start = read(&kind).ok_or_else(|| format!("unknown counter kind {kind:?}"))?;
    let trigger_at = start
        .checked_add(n)
        .ok_or_else(|| format!("counter threshold overflow for {kind:?}"))?;
    let mut rules = crash_rules()
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    rules.retain(|rule| rule.kind != kind);
    rules.push(CrashRule { kind, trigger_at });
    Ok(())
}

fn maybe_crash(kind: &str, value: u64) {
    let rule = {
        let mut rules = crash_rules()
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        rules
            .iter()
            .position(|rule| rule.kind == kind && value >= rule.trigger_at)
            .map(|idx| rules.remove(idx))
    };
    if let Some(rule) = rule {
        log::error!(
            "test-helpers: exiting after counter {} reached {}",
            rule.kind,
            value,
        );
        std::process::exit(99);
    }
}

fn crash_rules() -> &'static Mutex<Vec<CrashRule>> {
    CRASH_RULES.get_or_init(|| Mutex::new(Vec::new()))
}

fn counter(kind: &str) -> Option<&'static AtomicU64> {
    match kind {
        ACTION_BATCH_EXECUTE => Some(&ACTION_BATCH_EXECUTE_COUNT),
        ACTION_JOURNAL_WRITE => Some(&ACTION_JOURNAL_WRITE_COUNT),
        SEARCH_WRITE => Some(&SEARCH_WRITE_COUNT),
        SEARCH_INDEX => Some(&SEARCH_INDEX_COUNT),
        SEARCH_DELETE => Some(&SEARCH_DELETE_COUNT),
        SEARCH_CLEAR => Some(&SEARCH_CLEAR_COUNT),
        SEARCH_FLUSH => Some(&SEARCH_FLUSH_COUNT),
        _ => None,
    }
}
