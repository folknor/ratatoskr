# Action Service: Phase 3.1 Detailed Plan

## Goal

Replace all `String` error fields on `ActionOutcome` with a structured `ActionError` enum. This introduces the type system for error categorization and seeds the few classifications we can currently know (`NotImplemented` for stubs, `Build` for MIME, `Db` for database errors). Most provider errors remain `Unknown` because they arrive as opaque strings — better classification is incremental work per-provider, not a Phase 3.1 goal. Phase 3.4 will use `RemoteFailureKind` for enqueue decisions; Phase 3.1 provides the infrastructure.

## Current State

`ActionOutcome` has `String` errors:

```rust
pub enum ActionOutcome {
    Success,
    LocalOnly { remote_error: String },
    Failed { error: String },
}
```

97 occurrences across 18 files (14 action files in core + 1 in calendar + 3 app handlers). The strings are constructed via `format!()` or `.to_string()` from three error sources:

1. **DB errors** (~15 sites) — `"db lock: {e}"`, `"spawn_blocking: {e}"`, `"label lookup: {e}"`, `"draft persist: {e}"`. Source: `rusqlite::Error` or `PoisonError`.
2. **Provider creation errors** (~12 sites) — account lookup failure, credential decryption, client construction. Source: `String` from `create_provider()`.
3. **Provider operation errors** (~12 sites) — the actual API call failed. Source: `ProviderError::to_string()` or `String` from calendar/contact provider functions.
4. **Build errors** (2 sites) — MIME construction in send. Source: `SendError`.
5. **State errors** (3 sites) — draft state machine violation, missing contact identity, no calendar provider for account type.

## Design

### `ActionError` enum

```rust
#[derive(Debug, Clone)]
pub enum ActionError {
    /// Local database error (lock, query, constraint).
    Db(String),
    /// Remote provider operation failed.
    Remote {
        kind: RemoteFailureKind,
        message: String,
    },
    /// Resource not found (label, event, contact, draft, calendar).
    NotFound(String),
    /// State machine violation (e.g., draft already sending).
    InvalidState(String),
    /// Payload construction failed (MIME build, JSON serialization).
    Build(String),
}

/// Distinguishes retryable from permanent remote failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFailureKind {
    /// Network error, timeout, 5xx — worth retrying.
    Transient,
    /// 4xx, permission denied, invalid request — won't succeed on retry.
    Permanent,
    /// Provider write-back not yet implemented (stub).
    NotImplemented,
    /// Unknown completion — provider error couldn't be classified.
    Unknown,
}
```

### `user_message()` and `Display`

```rust
impl ActionError {
    /// User-facing summary for toast/status display.
    pub fn user_message(&self) -> String {
        match self {
            Self::Db(msg) => format!("Database error: {msg}"),
            Self::Remote { kind, message } => match kind {
                RemoteFailureKind::Transient => format!("Network error: {message}"),
                RemoteFailureKind::Permanent => format!("Server rejected: {message}"),
                RemoteFailureKind::NotImplemented => format!("Not yet supported: {message}"),
                RemoteFailureKind::Unknown => format!("Sync error: {message}"),
            },
            Self::NotFound(what) => format!("Not found: {what}"),
            Self::InvalidState(msg) => msg.clone(),
            Self::Build(msg) => format!("Build error: {msg}"),
        }
    }
}
```

Also implement `Display` (delegates to `user_message()`) and `std::error::Error`.

**`user_message()` is an intermediate step, not a polished user-safe boundary.** The messages still incorporate internal wording from provider errors and rusqlite messages. Phase 3.1 provides the structure (`ActionError` variants with categories) so that future work can refine the messages per-variant without changing the API. For now, `user_message()` is better than raw strings (it prepends context like "Network error:" or "Not found:") but is not fully user-safe copy.

### Convenience constructors

To minimize churn at the 97 call sites, provide constructors that match the current error patterns:

```rust
impl ActionError {
    /// Wrap a DB/lock/query error string.
    pub fn db(msg: impl Into<String>) -> Self { Self::Db(msg.into()) }

    /// Wrap a provider operation error. Defaults to Unknown kind
    /// since most provider errors are opaque strings.
    pub fn remote(msg: impl Into<String>) -> Self {
        Self::Remote { kind: RemoteFailureKind::Unknown, message: msg.into() }
    }

    /// Wrap a provider error with explicit kind.
    pub fn remote_with_kind(kind: RemoteFailureKind, msg: impl Into<String>) -> Self {
        Self::Remote { kind, message: msg.into() }
    }

    /// Provider write-back not yet implemented.
    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self::Remote { kind: RemoteFailureKind::NotImplemented, message: msg.into() }
    }

    pub fn not_found(msg: impl Into<String>) -> Self { Self::NotFound(msg.into()) }
    pub fn invalid_state(msg: impl Into<String>) -> Self { Self::InvalidState(msg.into()) }
    pub fn build(msg: impl Into<String>) -> Self { Self::Build(msg.into()) }
}
```

### Updated `ActionOutcome`

```rust
pub enum ActionOutcome {
    /// Local and remote both succeeded (or local-only-by-design succeeded).
    Success,
    /// Local succeeded, remote dispatch failed or was skipped.
    LocalOnly { reason: ActionError },
    /// The action failed entirely (local not applied).
    Failed { error: ActionError },
}
```

The helper methods (`is_success`, `is_local_only`, `is_failed`) stay.

### Classification strategy for `RemoteFailureKind`

Most provider errors arrive as opaque `String` (from `ProviderError::to_string()` or calendar/contact provider functions). Perfect classification requires inspecting the error string or the original error type. For Phase 3.1:

- **`create_provider()` failure** → `Unknown` (could be credential error, DB error, or network — can't tell from the string alone).
- **Provider operation failure** → `Unknown` (default). When `ProviderError` itself is available (rather than its string), the action can inspect it:
  - `ProviderError::Network(_)` → `Transient`
  - `ProviderError::Auth(_)` → `Permanent`
  - `ProviderError::Client(_)` → `Permanent`
  - `ProviderError::Server(_)` → `Transient`
  - Other → `Unknown`
- **Contact/calendar stubs** ("not yet wired to HTTP") → `NotImplemented`.
- **MIME build** → `Build` (not a `Remote` error at all).
- **DB lock/query** → `Db` (not a `Remote` error).

Phase 3.1 does NOT refactor `create_provider()` or `ProviderOps` methods to return structured errors. That's a larger cross-crate change. The classification works with what's available — mostly `Unknown` for opaque provider strings, with `NotImplemented` for known stubs. Better classification can be added incrementally as provider error types are refined.

## Implementation Steps

### Step 1: Define `ActionError`, `RemoteFailureKind`, and update `ActionOutcome`

In `crates/core/src/actions/outcome.rs`. Remove the Phase 1 doc comment ("String error fields are temporary"). Add the new types with `Display`, `Error`, convenience constructors, and `user_message()`.

### Step 2: Update all action functions in `crates/core/src/actions/*.rs`

14 files, ~46 construction sites. The migration is mechanical:

**DB/spawn_blocking errors** (current → new):
```rust
// Before:
.map_err(|e| format!("db lock: {e}"))?;
return ActionOutcome::Failed { error: e };

// After:
.map_err(|e| ActionError::db(format!("db lock: {e}")))?;
return ActionOutcome::Failed { error: e };
```

The `spawn_blocking` closures return `Result<T, String>`. Change them to return `Result<T, ActionError>`:

```rust
let local_result = tokio::task::spawn_blocking(move || {
    let conn = db.conn();
    let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
    // ...
})
.await
.map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
.and_then(|r| r);
```

**Provider creation errors:**
```rust
// Before:
Err(e) => return ActionOutcome::LocalOnly { remote_error: e };

// After:
Err(e) => return ActionOutcome::LocalOnly { reason: ActionError::remote(e) };
```

**Provider operation errors:**
```rust
// Before:
Err(e) => {
    let msg = e.to_string();
    ActionOutcome::LocalOnly { remote_error: msg }
}

// After:
Err(e) => {
    let msg = e.to_string();
    ActionOutcome::LocalOnly { reason: ActionError::remote(msg) }
}
```

**Contact stubs:**
```rust
// Before:
Err("Google contact write-back not yet wired to HTTP".to_string())

// After:
Err(ActionError::not_implemented("Google contact write-back not yet wired to HTTP"))
```

**Send MIME build:**
```rust
// Before:
Err(e) => return ActionOutcome::Failed { error: format!("MIME build: {e}") },

// After:
Err(e) => return ActionOutcome::Failed { error: ActionError::build(format!("{e}")) },
```

**Label/event/calendar/contact lookups via `query_row`:**

`query_row` can fail with `QueryReturnedNoRows` (genuinely not found) or other `rusqlite::Error` variants (DB corruption, lock timeout, malformed row). These must be classified differently:

```rust
// Before:
.map_err(|e| format!("label lookup: {e}"))?;

// After (WRONG — maps all errors to NotFound):
.map_err(|e| ActionError::not_found(format!("label: {e}")))?;

// After (CORRECT — distinguish not-found from DB errors):
.map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
        ActionError::not_found("label not found for this account")
    }
    other => ActionError::db(format!("label lookup: {other}")),
})?;
```

This applies to ~5 lookup sites: label metadata, event metadata, calendar remote ID, contact identity, and draft lookup. Each `query_row` call must match on the error variant.

### Step 3: Update calendar action functions in `crates/calendar/src/actions.rs`

18 construction sites. Same patterns as Step 2. The `dispatch_write_back` and `dispatch_delete` functions return `Result<(), String>` — change to `Result<(), ActionError>`. Provider functions that return `Result<(), String>` (e.g., `jmap_contacts_push_update`) need an intermediate `.map_err(ActionError::remote)` at the call site since they won't be changed to return `ActionError` directly.

### Step 4: Update app handlers

4 files, ~10 match sites:

**`handle_action_completed`** (`commands.rs`):
```rust
// Before:
ActionOutcome::Failed { error } => Some(error.as_str()),

// After:
ActionOutcome::Failed { error } => Some(error.user_message()),
```

The `errors.join("; ")` pattern stays but operates on `String` from `user_message()` instead of raw error strings.

**`handle_send_completed`** (`pop_out.rs`):
```rust
// Before:
ActionOutcome::Failed { error } | ActionOutcome::LocalOnly { remote_error: error } => {
    state.status = Some(format!("Send failed: {error}"));

// After:
ActionOutcome::Failed { error } | ActionOutcome::LocalOnly { reason: error } => {
    state.status = Some(format!("Send failed: {}", error.user_message()));
```

**`calendar_outcome_to_result`** (`calendar.rs`):
```rust
// Before:
ActionOutcome::Failed { error } => Err(error),

// After:
ActionOutcome::Failed { error } => Err(error.user_message()),
```

**Contact handlers** (`contacts.rs`): The save handler routes `Failed` to `ContactSaved(Err(error))` and the delete handler routes `Failed` to `ContactDeleted(Err(error))`. These already surface errors via the settings message system. However, the settings UI currently no-ops on both success and failure results (settings/update.rs:613-614 — both arms are empty). Phase 3.1 updates the error strings to use `.user_message()` in the handler; making the settings UI actually display these errors is a UI concern beyond Phase 3.1's scope but should be noted.

### Step 5: Verify

- `cargo check --workspace`
- `cargo clippy -p ratatoskr-core -p ratatoskr-calendar -p app`
- Grep for `remote_error:` — should be zero (renamed to `reason:`).
- Grep for `error: String` in outcome.rs — should be zero.
- Grep for `ActionOutcome::Failed { error: format!` — should be zero (all use `ActionError` constructors).

## What This Produces

- Modified `crates/core/src/actions/outcome.rs` — `ActionError`, `RemoteFailureKind`, updated `ActionOutcome`
- Modified all 14 action files in `crates/core/src/actions/` — `ActionError` constructors
- Modified `crates/calendar/src/actions.rs` — same
- Modified 4 app handler files — `.user_message()` for display

## Exit Criteria

1. No `String` error fields on `ActionOutcome`. All `error: String` replaced with `error: ActionError`, all `remote_error: String` replaced with `reason: ActionError`.
2. `ActionError` has 5 variants: `Db`, `Remote`, `NotFound`, `InvalidState`, `Build`.
3. `RemoteFailureKind` has 4 variants: `Transient`, `Permanent`, `NotImplemented`, `Unknown`.
4. Contact stubs use `NotImplemented`. MIME build uses `Build`. DB errors use `Db`. Provider errors use `Remote` (mostly `Unknown` kind for now — better classification is incremental).
5. `user_message()` returns meaningful user-facing text for all variants.
6. App handlers use `user_message()` for display — no raw string access.
7. Workspace compiles and passes clippy.

## What Phase 3.1 Does NOT Do

- **Classify all provider errors precisely.** Most provider errors arrive as opaque strings. `Unknown` is the default kind. Better classification (inspecting `ProviderError` variants) is incremental work that can happen per-provider.
- **Change `ProviderError` or `create_provider` signatures.** That's a cross-crate refactor beyond Phase 3.1's scope.
- **Add `retryable: bool` to `LocalOnly`.** That's Phase 3.2 (outcome semantics).
- **Change outcome variant semantics.** `Success`/`LocalOnly`/`Failed` keep their current meanings. Phase 3.2 refines them.
