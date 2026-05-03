# Phase 1 Implementation Discrepancies

Current gaps between `phase-1-plan.md` and the in-tree implementation as of this review. This document is a punch list of work outstanding for Phase 1 to close cleanly. Items get removed as they are fixed; this is not a historical record.

The implementation lives across `crates/service-api/`, `crates/service/`, `crates/app/src/service_client.rs`, `crates/app/src/service_subscription.rs`, and the wire-up in `crates/app/src/{main,app,handlers/core,message,subscription,update}.rs`. `brokkr check -p service-api`, `brokkr check -p service`, and `brokkr check -p app` all pass.

## Critical gaps vs. plan

### 1. Windows parent-death is unimplemented

`crates/service/src/parent_death/windows.rs` is a stub:

```rust
pub(super) fn configure_command(_command: &mut tokio::process::Command) -> io::Result<()> {
    Ok(())
}
```

Plan section 13 requires a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` plus `assign_process_to_job`. Without it, killing the UI on Windows leaves an orphan Service. Plan explicitly scopes Windows into v1 ("v1: Linux + Windows; macOS deferred to post-1.0").

### 2. Windows stdio corruption defense is unimplemented

`crates/service/src/stdio_defense.rs` only has a `cfg(unix)` impl. `lib.rs` falls through to `tokio::io::stdin()` / `tokio::io::stdout()` on Windows (`crates/service/src/lib.rs:48-53`). Plan section 11 requires "Windows: equivalent via `DuplicateHandle` + `SetStdHandle` against `NUL`". A stray `println!` in any transitive dep will desync the JSON-RPC framing on Windows.

### 3. NotificationClass routing does not match the plan

`crates/app/src/service_client.rs`:

```rust
match notification.class() {
    MustDeliver => { let _ = notifications.send(notification).await; }
    Coalesce { .. } | Drop => { let _ = notifications.try_send(notification); }
}
```

Plan section 8:

- `Coalesce { key }`: latest-wins on the enqueue side - find existing entry for the same key and overwrite. Implementation does no key lookup and does not coalesce.
- `Drop`: drop oldest under queue pressure. Implementation drops the newest (`try_send` rejects on full).

The empty `Notification` enum means nothing exercises this in Phase 1, but the framework is supposed to "land so Phase 2 plugs in cleanly the next day"; today Phase 2 will rewrite this routing immediately.

### 6. Notification framework is not round-trip-tested

`crates/service-api/src/notification.rs` has `pub enum Notification {}` with `#[serde(tag = "method", content = "params")]` and `Serialize` / `Deserialize` derives. The Phase 1 framework is supposed to "land so Phase 2 plugs in cleanly the next day", but no test ever sends a notification through the wire end-to-end with a feature-gated test variant. The synthetic JSON object built in `parse_service_message` (`{"method": ..., "params": ...}`) needs to round-trip cleanly with `tag = "method", content = "params"` semantics; that contract is currently unverified.

## Missing tests promised by the plan

In-tree tests today:

- `crates/service-api/src/error.rs`: `ServiceError` round-trips through `JsonRpcErrorObject.data` (3 unit tests).
- `crates/service/tests/dispatch_in_process.rs`: ping happy path, malformed JSON, oversize frame, EOF, concurrent fan-out, invalid UTF-8 returns parse-error and loop continues, invalid request correlates parse-error to extracted id (7 tests).
- `crates/app/tests/service_subprocess.rs`: spawn + ping + shutdown via the wire (1 test).

Promised by plan section 13 / 20-21 but not present:

- Panicking handler returns `ServiceError::Panic`; loop continues. (Requires a `#[cfg(test)]` panic-injecting handler.)
- Version mismatch: test-only handler returning a wrong version; client reports `ClientError::VersionMismatch`.
- In-flight semaphore cap: 200 concurrent slow handlers; assert at most 64 run at once; queued handlers complete eventually; heartbeat bypasses.
- Spawn-failure: point `current_exe` at a non-existent binary; assert clear error, no hang.
- `BoundedLineReader` peak-buffer bound: assert peak buffer size stays bounded under a 1 GiB no-newline payload.
- Spawn + drop `ServiceClient` without `shutdown()`; child exits within 1 s; no orphan.
- Linux SIGKILL of the parent process: subprocess exits within 2 s via PR_SET_PDEATHSIG.
- Stdout corruption defense: test-only build path that calls `println!` from a transitive dep; assert the JSON-RPC framing on the saved-FD stdout is unaffected.
- Heartbeat survives a single timeout: no automated coverage of the "missed beat keeps ticking" path.
- `PendingGuard` evicts on cancel: no automated coverage of the request-future-dropped-mid-flight path.

Plan section 22 closes with: "Test-only handlers (panic-injecting, version-mismatch) are `#[cfg(test)]`-gated so they cannot ship." Those handlers do not exist yet.

The Phase 1 promotion criterion is "the integration test is green in CI"; the in-tree set is roughly half the promised set.

## Smaller correctness issues

### 7. `dirs::data_dir()` default writes Service logs to the prod data dir during dev

`crates/service/src/lib.rs:69-73` falls back to `dirs::data_dir().join("org.folknor.ratatoskr")` when `--app-data-dir` isn't provided. `cargo run -p app -- --service` (suggested as a smoke check in plan section 7) writes its log + `clean_shutdown` sentinel into the production data dir, not the dev one. A `target/`-rooted default would isolate dev runs.

### 8. `RequestParams::params_value` always returns `{}`

`crates/service-api/src/request.rs:36-40`. Phase 2 will need either struct-shaped enum variants on `RequestParams` or a separate per-method type with `Serialize` / `Deserialize`. Worth restructuring now while the surface is small (two methods).

### 9. `ServiceError::AnotherInstanceRunning` is dead code

`crates/service-api/src/error.rs:14-15`. Single-instance lock is Phase 1.5 work. Either remove the variant now and re-add in 1.5, or accept that this code path is unverified. Today the `From<ServiceError>` impl maps it to `JsonRpcErrorObject::internal` with a hardcoded "another instance is running" string, none of which is reachable.

### 10. `panic_message` only handles `&str` and `String` payloads

`crates/service/src/dispatch.rs:162-170`. Custom panic types (e.g. `panic!(value)` with arbitrary types) report `"unknown panic payload"`. Acceptable but documents a real loss of detail.

### 11. Smoke test does not exercise `ServiceClient` Drop

`crates/app/tests/service_subprocess.rs` writes requests and reads responses directly through `BoundedLineReader`, never spawning a real `ServiceClient`. The plan's "Spawn + drop without shutdown" path needs the actual `ServiceClient` Drop ordering exercised against a real subprocess. Today nothing tests that path end-to-end.

### 12. Smoke test does not clean up `target/service-smoke-{pid}/`

`crates/app/tests/service_subprocess.rs:87-91` creates a per-pid directory under `target/` and never removes it. Each run leaves a clean-shutdown sentinel and (eventually) Service log files behind. Trivial; flag for tidying when tests are extended.

## Out of scope (do not address in Phase 1)

These are explicitly deferred per plan section "Out of scope":

- Respawn-on-crash (Phase 1.5).
- Tray icon, autostart, daemon promotion.
- JSON-RPC schema versioning (pin format-version-1; bump method names later).
- Authentication / authorization (same trust domain).
- Schema migrations and encryption-key relocation (Phase 1.5).
- Single-instance file lock (Phase 1.5).
- macOS parent-death (post-1.0; design retained in `problem-statement.md`).
