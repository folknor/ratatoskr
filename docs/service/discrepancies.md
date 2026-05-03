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

## Missing tests promised by the plan

In-tree tests today:

- `crates/service-api/src/error.rs`: `ServiceError` round-trips through `JsonRpcErrorObject.data` (3 unit tests).
- `crates/service-api/src/notification.rs`: `Notification` round-trips through serde + `parse_service_message`; class + method_name lookup (4 unit tests, gated on a `#[cfg(test)] TestEcho` variant).
- `crates/service/tests/dispatch_in_process.rs`: ping happy path, malformed JSON, oversize frame, EOF, concurrent fan-out, invalid UTF-8 returns parse-error and loop continues, invalid request correlates parse-error to extracted id (7 tests).
- `crates/app/src/notification_queue.rs`: Coalesce replaces existing entry by key, Coalesce preserves slot when replacing, Drop evicts oldest under pressure, MustDeliver blocks producer when full, close unblocks recv with None, cross-class FIFO is preserved (6 unit tests over a `Classifiable` mock).
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

### 11. Smoke test does not exercise `ServiceClient` Drop

`crates/app/tests/service_subprocess.rs` writes requests and reads responses directly through `BoundedLineReader`, never spawning a real `ServiceClient`. The plan's "Spawn + drop without shutdown" path needs the actual `ServiceClient` Drop ordering exercised against a real subprocess. Today nothing tests that path end-to-end.

## Out of scope (do not address in Phase 1)

These are explicitly deferred per plan section "Out of scope":

- Respawn-on-crash (Phase 1.5).
- Tray icon, autostart, daemon promotion.
- JSON-RPC schema versioning (pin format-version-1; bump method names later).
- Authentication / authorization (same trust domain).
- Schema migrations and encryption-key relocation (Phase 1.5).
- Single-instance file lock (Phase 1.5).
- macOS parent-death (post-1.0; design retained in `problem-statement.md`).
