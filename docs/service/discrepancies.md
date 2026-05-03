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

`crates/app/src/service_client.rs:355-367`:

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

### 4. `ClientError` flattens every wire error to `Internal`

`crates/app/src/service_client.rs:351-353`:

```rust
fn client_error_from_rpc(error: JsonRpcErrorObject) -> ClientError {
    ClientError::Service(ServiceError::Internal(error.message))
}
```

Server-side `ServiceError::Panic { method, message }`, `InvalidParams`, `UnknownMethod`, etc. are serialized correctly into structured `JsonRpcErrorObject`, then collapsed back into `ServiceError::Internal` at the client. The error code is also discarded. Plan section 4 calls for the typed `request<R, P>()` wrapper to deserialize into the expected error variants; the in-tree pending map type (`oneshot::Sender<Result<Value, ClientError>>` instead of `Result<Value, ServiceError>`) is the proximate cause.

### 5. Service-side parse-errors never reach the requester

A parse-error response built by the dispatch loop carries `id: null` (`crates/service/src/dispatch.rs:60`, `dispatch.rs:104-110`). On the client, `parse_service_message` parses this as `ParsedServiceMessage::Response { id: None, ... }`, and `reader_task` logs "uncorrelated response" and drops it (`crates/app/src/service_client.rs:312-314`). The requester sits in `pending` until its per-method timeout fires.

The plan's failure-mode test ("malformed JSON returns parse-error and Service stays up") needs the client side to surface this. Either correlate parse-errors back to the most recent in-flight id (best-effort), or have the Service include a `data` field with the failed id when it can be recovered, or make the client treat any error response with `id=null` as "fail the most recent pending entry that posted bytes after the last response". None are perfect; the current behavior (silently drop) is the worst option.

### 6. Drop ordering omits the await-with-deadline step and uses `std::thread::sleep`

`crates/app/src/service_client.rs:265-285`:

```rust
abort_handle(&self.reader_handle);
abort_handle(&self.heartbeat_handle);
abort_handle(&self.writer_handle);
let _ = self.stdin_tx.take();

let started = Instant::now();
while started.elapsed() < Duration::from_millis(200) {
    if self.try_wait_child() { break; }
    std::thread::sleep(Duration::from_millis(10));
}
if !self.try_wait_child() { self.kill_child(); }
fail_pending(&self.pending);
```

Two issues:

- Plan section 5 step 2 ("Await tasks with a short deadline (200 ms)") is omitted entirely. The plan-sketch comment notes that awaiting from sync `Drop` is awkward, but the abort handles must drop before the underlying `ChildStdin` / `ChildStdout` close, and aborts only progress when the runtime gets a chance to drive the tasks.
- The 200 ms wait uses `std::thread::sleep`, blocking the calling thread without yielding to the runtime. If `Drop` runs from inside a tokio worker (e.g. the iced quit path), the aborted tasks may not make any progress during the wait, and the path falls through to `kill_child()` (SIGKILL) every time. Polling via `tokio::task::block_in_place` + `tokio::time::sleep`, or simply holding a `tokio::runtime::Handle` and `block_on`-ing a timeout future, would actually let the abort propagate.

### 7. Linux Service stdio uses `tokio::fs::File` for pipes

`crates/service/src/stdio_defense.rs:24-26`:

```rust
let stdin = unsafe { std::fs::File::from_raw_fd(stdin_fd) };
let stdout = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
Ok((tokio::fs::File::from_std(stdin), tokio::fs::File::from_std(stdout)))
```

`tokio::fs::File` is for regular files: it dispatches every read / write onto a blocking-pool thread. For pipes you want epoll-driven readiness via something like `tokio::io::unix::AsyncFd` (or the explicit `tokio::process::ChildStdin` / `ChildStdout` types if you can keep them attached). As written, every `BoundedLineReader::next_line` call holds a blocking-pool thread for the duration of the read.

### 8. Invalid UTF-8 from a single byte kills the dispatch loop

`crates/service-api/src/framing.rs` returns `FrameError::InvalidUtf8` from `BoundedLineReader::next_line`. The dispatch loop in `crates/service/src/dispatch.rs:62-65` matches only `FrameError::TooLarge` specially; all other errors hit the generic arm and `break`:

```rust
Err(FrameError::TooLarge) => {
    log::warn!("rejecting oversized frame");
    send_error(&out_tx, None, JsonRpcErrorObject::parse_error("frame too large")).await;
}
Err(error) => {
    log::warn!("service frame read failed: {error}");
    break;
}
```

A single garbled byte from any producer takes the whole Service down. UTF-8 failures should follow the parse-error path: emit a parse-error response and keep reading.

### 9. Heartbeat exits permanently after a single timeout

`crates/app/src/service_client.rs:401-410`:

```rust
match result {
    Ok(value) => match serde_json::from_value::<HealthPingResponse>(value) {
        Ok(_) => log::debug!("service heartbeat ok in {:?}", started.elapsed()),
        Err(error) => log::warn!("service heartbeat decode failed: {error}"),
    },
    Err(error) => {
        log::warn!("service heartbeat failed: {error}");
        return;
    }
}
```

Any error - including a single `Timeout` - terminates the heartbeat task. Plan section 12 ("every 30 s; logs round-trip + missed beats") implies continued logging across missed beats. Today, after one missed beat, the loop is silent forever. Either keep ticking and logging until `stdin_tx` actually fails, or document that heartbeat is one-shot-on-failure (and adjust the plan).

### 10. Notification framework is not round-trip-tested

`crates/service-api/src/notification.rs` has `pub enum Notification {}` with `#[serde(tag = "method", content = "params")]` and `Serialize` / `Deserialize` derives. The Phase 1 framework is supposed to "land so Phase 2 plugs in cleanly the next day", but no test ever sends a notification through the wire end-to-end with a feature-gated test variant. The synthetic JSON object built in `parse_service_message` (`{"method": ..., "params": ...}`) needs to round-trip cleanly with `tag = "method", content = "params"` semantics; that contract is currently unverified.

## Missing tests promised by the plan

In-tree tests today:

- `crates/service/tests/dispatch_in_process.rs`: ping happy path, malformed JSON, oversize frame, EOF, concurrent fan-out (5 tests).
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

Plan section 22 closes with: "Test-only handlers (panic-injecting, version-mismatch) are `#[cfg(test)]`-gated so they cannot ship." Those handlers do not exist yet.

The Phase 1 promotion criterion is "the integration test is green in CI"; the in-tree set is roughly half the promised set.

## Smaller correctness issues

### 11. The `Shutdown` request branch bypasses panic safety

`crates/service/src/dispatch.rs:91-97` handles `Shutdown` inline in the dispatch loop (calls `lifecycle.drain().await` and `serde_json::to_value(...)` directly). Other handlers run inside `dispatch_with_panic_safety` (`AssertUnwindSafe(...).catch_unwind()`). A panic in `drain()` (unlikely today, but more likely once Phase 1.5 wires real flush work into it) would take the dispatch loop down without producing a panic-converted response.

### 12. `ServiceLifecycle::drain` is run-once, return-value lies on second call

`crates/service/src/lifecycle.rs:42-57`:

```rust
pub(crate) async fn drain(&self) -> bool {
    if self.drained.swap(true, Ordering::SeqCst) {
        return true;
    }
    ...
}
```

Subsequent calls return `true` regardless of whether the first call succeeded. The dispatch loop's outer drain (after the loop breaks) and the SIGTERM handler can both race the inline `Shutdown` branch's drain; whoever loses gets `true` even if flushing failed. Cosmetic in Phase 1 (no real flushing yet) but worth fixing before 1.5.

### 13. UI-side `request_value` does not evict on send failure into oneshot drop

`crates/app/src/service_client.rs:177-191`: on `RequestTimeoutKind::Finite` timeout, `pending.remove(&id)` runs. Good. But if the oneshot receiver itself drops (e.g. caller dropped the future before timeout), the entry stays in `pending` until the reader fills it or the Service crashes. The `pending` map can leak entries under cancellation. Plan section 6 says "Expired requests evict their pending entry" - the same hygiene should apply to cancelled-future cleanup.

### 14. `dirs::data_dir()` default writes Service logs to the prod data dir during dev

`crates/service/src/lib.rs:69-73` falls back to `dirs::data_dir().join("org.folknor.ratatoskr")` when `--app-data-dir` isn't provided. `cargo run -p app -- --service` (suggested as a smoke check in plan section 7) writes its log + `clean_shutdown` sentinel into the production data dir, not the dev one. A `target/`-rooted default would isolate dev runs.

### 15. Reader and Drop both call `fail_pending`

`crates/app/src/service_client.rs:323` (reader EOF/error path) and `service_client.rs:283` (Drop). Two paths race to remove and reject pending senders. Functional, but the plan puts this responsibility solely on Drop ("Drain pending; reject every outstanding sender"). Pick one site - the reader can leave it to Drop, since closing stdin will cause the writer task to exit and Drop runs on `ServiceClient` cleanup.

### 16. `RequestParams::params_value` always returns `{}`

`crates/service-api/src/request.rs:36-40`. Phase 2 will need either struct-shaped enum variants on `RequestParams` or a separate per-method type with `Serialize` / `Deserialize`. Worth restructuring now while the surface is small (two methods).

### 17. `ServiceError::AnotherInstanceRunning` is dead code

`crates/service-api/src/error.rs:14-15`. Single-instance lock is Phase 1.5 work. Either remove the variant now and re-add in 1.5, or accept that this code path is unverified. Today the `From<ServiceError>` impl maps it to `JsonRpcErrorObject::internal` with a hardcoded "another instance is running" string, none of which is reachable.

### 18. `panic_message` only handles `&str` and `String` payloads

`crates/service/src/dispatch.rs:162-170`. Custom panic types (e.g. `panic!(value)` with arbitrary types) report `"unknown panic payload"`. Acceptable but documents a real loss of detail.

### 19. Smoke test does not exercise `ServiceClient` Drop

`crates/app/tests/service_subprocess.rs` writes requests and reads responses directly through `BoundedLineReader`, never spawning a real `ServiceClient`. The plan's "Spawn + drop without shutdown" path needs the actual `ServiceClient` Drop ordering exercised against a real subprocess. Today nothing tests that path end-to-end.

### 20. Smoke test does not clean up `target/service-smoke-{pid}/`

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
