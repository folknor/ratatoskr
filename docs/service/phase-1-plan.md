# The Service - Phase 1 Plan: Process Boundary Scaffolding

Detailed implementation plan for Phase 1 of `implementation-roadmap.md`. This document is meant to be reviewable end-to-end before any code is written; all subsequent phases get their own equivalent document at the time they're tackled.

## Context

Phase 1 lands the bare scaffolding for the two-process architecture defined in `problem-statement.md`. Specifically: a child process exists, the UI spawns it on start, the two exchange a single JSON-RPC method (`health.ping`), and lifecycle (start, heartbeat, clean shutdown, parent-death detection) all work cleanly. **No real functionality moves across the boundary in this phase.** Sync, the action service, the Tantivy writer, and blob store writes all stay where they are today; the scaffolding just makes the future relocations possible without rewriting infrastructure.

The deliverable is small and well-scoped on purpose: every later phase plugs into this scaffold, so getting the scaffold right is worth more than getting it big.

## Scope

### In scope

1. **Single-binary, dual-mode dispatch.** The existing `ratatoskr` binary gains a `--service` flag. With the flag, it runs the Service entry point and exits. Without, it boots the iced app as today.
2. **Two new workspace crates:**
   - `crates/service-api/` - shared types: `Request`, `Response`, `Notification` enums, `ServiceError`, `PROTOCOL_VERSION`, framing helpers (`write_message`, `read_message`). Phase 1 surface: `health.ping` + `shutdown` request.
   - `crates/service/` - runtime. Two entry points:
     - `run_service()` - production entry, wires real stdin/stdout.
     - `run_service_with_io<R: AsyncRead, W: AsyncWrite>(stdin, stdout)` - testable entry, generic over IO.
3. **`ServiceClient` in the app crate.** Spawns subprocess, manages stdio pipes, correlates request IDs, exposes typed `request<R, P>(method, params, timeout)`. Background reader task. Background stdout-writer task with bounded queue (the dispatch loop never blocks on stdout).
4. **Pending-request map.** `DashMap<u64, oneshot::Sender<Result<serde_json::Value, ServiceError>>>` - **not** an untagged response enum. The typed `request()` wrapper deserializes the value into the expected response type after correlating by id. Drop impl drains and rejects every outstanding sender.
5. **Per-request timeouts.** Default 5 s; `shutdown` gets 30 s; `index.rebuild` (when it lands later) gets infinite. Expired requests evict their pending entry.
6. **Bounded notification channel.** Cap 1024. Phase 1 has no notifications yet, but the channel + the writer-task + the cap land here so later phases plug in cleanly.
7. **Frame size cap.** 4 MiB per line; oversize frames rejected at the framing layer.
8. **Heartbeat.** UI sends `health.ping` every 30 s; logs round-trip + missed beats. No respawn (Phase 1.5).
9. **Parent-death detection (race-free per platform).**
   - Linux: `pre_exec` hook calling `prctl(PR_SET_PDEATHSIG, SIGTERM)` + post-prctl `getppid()` check at startup. If the parent already died before the hook took effect, exit immediately.
   - macOS: `kqueue` with `EVFILT_PROC` + `NOTE_EXIT` registered against the parent PID at start. Fires when *that specific process* exits.
   - Windows: `OpenProcess` against parent PID + `WaitForSingleObject` on the resulting HANDLE. Race-free against PID reuse.
10. **Clean shutdown.** `shutdown` is a **request**. UI awaits the response with a 30 s timeout, then SIGTERM, then SIGKILL after another 5 s.
11. **Version handshake.** First `health.ping` after spawn asserts `response.version == PROTOCOL_VERSION`; mismatch is fatal boot error with a clear "binary mismatch" message.
12. **Panic safety.** Every handler runs inside `AssertUnwindSafe(...).catch_unwind()`. Panics return `ServiceError::Panic { method, message }`; the dispatch loop continues. A panicking PDF extractor in Phase 7 won't kill the Service.
13. **File-based logging.** Service writes to `<app_data>/logs/service.log` with simple size-based rolling (~10 MB cap, keep 3). stderr stays for `cargo run` debugging. Tagged `[service]` / `[ui]` prefixes for disambiguation in the interleaved console output.
14. **Integration tests via in-process dispatch.** `tokio::io::duplex` driving `run_service_with_io` - no subprocess spawn, no test-binary scaffolding. Real-subprocess test is deferred to Phase 2+ when the IPC surface justifies it.
15. **Real failure-mode tests.** EOF-during-pending-request, malformed JSON, concurrent ping fan-out (id correlation), version mismatch, spawn failure, panicking handler, oversize frame rejection.

### Out of scope

- Any actual functionality moving across the boundary.
- Respawn-on-crash. Phase 1.5.
- Tray icon, autostart, daemon promotion.
- Schema versioning of the JSON-RPC protocol. Pin format-version-1 in v1; bump method names if the contract changes later.
- Authentication / authorization between UI and Service. Same trust domain.
- Real-subprocess integration tests. In-process dispatch via `run_service_with_io` covers the contract; subprocess tests come when there are real handlers to validate cross-process.
- Schema migrations + encryption-key relocation - those are Phase 1.5, not Phase 1.

## Architecture

### Workspace layout after Phase 1

```
crates/
  app/                   # existing, gains --service dispatch + ServiceClient
  service/               # NEW - Service runtime
    src/
      lib.rs             # pub fn run_service() -> entry point
      dispatch.rs        # JSON-RPC dispatch loop
      handlers/
        mod.rs
        health.rs        # health.ping handler
      lifecycle.rs       # parent-death detection, shutdown coordination
      logging.rs         # tagged stderr logger setup
  service-api/           # NEW - shared types
    src/
      lib.rs             # pub use the enums
      request.rs         # Request enum
      response.rs        # Response enum + typed result types
      notification.rs    # Notification enum
      error.rs           # ServiceError type
      version.rs         # PROTOCOL_VERSION constant
```

Both new crates are workspace members. `app` depends on both. `service` depends on `service-api`. Neither crate depends on `app`.

### Dataflow on the wire

**Request / response (synchronous-feeling, request id correlated):**

```
UI:    {"jsonrpc":"2.0","id":42,"method":"health.ping","params":{}}\n
Svc:   {"jsonrpc":"2.0","id":42,"result":{"version":1,"pid":12345,"uptime_ms":1234}}\n
```

**Notification (one-way, fire-and-forget):**

```
UI:    {"jsonrpc":"2.0","method":"shutdown","params":{}}\n
```

(No `id` field on notifications, per JSON-RPC 2.0 spec.)

**Framing:** newline-delimited JSON. Reader uses `tokio::io::AsyncBufReadExt::read_line`. Per-line size is uncapped in v1; if it becomes a concern (large bytes payloads later) we'll switch to length-prefixed framing.

### Type definitions (sketch; final shape settles in code)

In `service-api/src/lib.rs`:

```rust
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;  // 4 MiB

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RequestParams {
    HealthPing,
    Shutdown,        // request, not notification - we await an ack
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HealthPingResponse {
    pub version: u32,
    pub pid: u32,
    pub uptime_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ShutdownResponse {
    pub flushed_ok: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Notification {
    // empty in Phase 1 - the framework lands but nothing emits notifications yet
}

#[derive(Debug, serde::Serialize, serde::Deserialize, thiserror::Error)]
pub enum ServiceError {
    #[error("handler panic in {method}: {message}")]
    Panic { method: String, message: String },
    #[error("invalid params for {method}: {message}")]
    InvalidParams { method: String, message: String },
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

**No untagged response enum.** Responses go through the wire as `serde_json::Value`; the typed `request<R, P>()` wrapper deserializes into the expected `R: DeserializeOwned` after correlating by id. This avoids the silent-misroute trap when two methods have structurally similar response types.

The wire envelope (request/response/notification) is JSON-RPC 2.0:

```rust
// Outgoing request
{"jsonrpc":"2.0","id":42,"method":"health.ping","params":{}}\n

// Incoming response
{"jsonrpc":"2.0","id":42,"result":{"version":1,"pid":12345,"uptime_ms":1234}}\n
// or
{"jsonrpc":"2.0","id":42,"error":{"code":-32603,"message":"...","data":{...}}}\n

// Notification (no id, no response)
{"jsonrpc":"2.0","method":"sync.progress","params":{...}}\n
```

`service-api` exposes a `write_message<T: Serialize, W: AsyncWrite>(value: &T, w: &mut W)` helper that uses compact serialization and appends a single `\n`. The crate forbids direct use of `serde_json::to_string_pretty` at the API level. One careless pretty-print would desync the framing.

Realistic LOC for the framing layer with proper error handling (parse errors, frame-size rejection, EOF handling, panic catching, partial reads, write timeouts): 200-300 LOC, not 50.

### `ServiceClient` (UI-side) sketch

In `crates/app/src/service_client.rs`:

```rust
pub struct ServiceClient {
    child: tokio::sync::Mutex<tokio::process::Child>,
    stdin_tx: tokio::sync::mpsc::Sender<Vec<u8>>,  // bounded channel into the writer task
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ServiceError>>>>,
    next_id: AtomicU64,
    notifications_tx: tokio::sync::mpsc::Sender<Notification>,  // bounded, cap 1024
    default_timeout: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("service error: {0}")]
    Service(#[from] ServiceError),
    #[error("request timeout")]
    Timeout,
    #[error("service crashed")]
    ServiceCrashed,
    #[error("not connected")]
    NotConnected,
    #[error("protocol version mismatch: ui={ui}, service={service}")]
    VersionMismatch { ui: u32, service: u32 },
    #[error("response deserialize: {0}")]
    Deserialize(#[from] serde_json::Error),
}

impl ServiceClient {
    pub async fn spawn() -> Result<Self, ClientError>;
    pub async fn request<R: DeserializeOwned>(
        &self,
        params: RequestParams,
        timeout: Option<Duration>,
    ) -> Result<R, ClientError>;
    pub async fn shutdown(&self) -> Result<(), ClientError>;
    pub fn subscribe_notifications(&self) -> tokio::sync::mpsc::Receiver<Notification>;
}

impl Drop for ServiceClient {
    fn drop(&mut self) {
        // Drain pending; reject every outstanding sender with NotConnected
        // so dropped clients don't leave hung waiters.
    }
}
```

Background tasks owned by the `ServiceClient`:
- **Reader task**: parses lines from the child's stdout, dispatches responses to `pending` (by id) or notifications to `notifications_tx` (no id). On EOF, fails every pending sender with `ClientError::ServiceCrashed`.
- **Writer task**: drains `stdin_tx` and writes to the child's stdin. Bounded; if the child can't keep up the channel applies backpressure to callers.
- **Heartbeat task**: every 30 s sends `RequestParams::HealthPing`; logs round-trip time; logs warning on missed beat. (No respawn until Phase 1.5.)

### Process spawn

```rust
let mut cmd = tokio::process::Command::new(std::env::current_exe()?);
cmd.arg("--service")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::inherit())
    .kill_on_drop(true);

#[cfg(target_os = "linux")]
unsafe {
    use std::os::unix::process::CommandExt;
    cmd.pre_exec(|| {
        // Set PR_SET_PDEATHSIG so SIGTERM fires when the parent (UI) thread exits.
        // The Service code re-checks getppid() at startup to close the
        // "parent died before this hook ran" race.
        let r = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        if r != 0 { return Err(std::io::Error::last_os_error()); }
        Ok(())
    });
}

let child = cmd.spawn()?;
```

On macOS, the spawning thread instead registers a `kqueue NOTE_EXIT` against the parent PID after `fork()` (or, simpler, the Service-side spawn handler does so at startup). On Windows, the Service-side startup calls `OpenProcess` against the parent PID and parks a task on `WaitForSingleObject`.

### Service-side dispatch loop

```rust
pub async fn run_service() -> ! {
    setup_logging();
    install_panic_hook();
    spawn_parent_death_watcher();   // platform-gated; race-free per platform
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let exit_code = run_service_with_io(stdin, stdout).await;
    std::process::exit(exit_code);
}

pub async fn run_service_with_io<R, W>(reader: R, writer: W) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // Bounded channel from dispatch -> writer task.
    // Dispatch never blocks on stdout; writer task drains.
    let (out_tx, out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1024);
    let writer_handle = tokio::spawn(writer_task(writer, out_rx));

    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > MAX_FRAME_BYTES {
            // Reject oversize frame; log but don't crash.
            log_oversize_frame(line.len());
            continue;
        }
        match parse_envelope(&line) {
            Ok(Envelope::Request { id, params }) => {
                let out_tx = out_tx.clone();
                tokio::spawn(async move {
                    let result = dispatch_with_panic_safety(params).await;
                    let response = build_response(id, result);
                    let _ = out_tx.send(response).await;
                });
            }
            Ok(Envelope::Notification(_)) => { /* none in Phase 1 */ }
            Err(parse_err) => {
                // Send a parse-error response with id=null per JSON-RPC spec.
                let _ = out_tx.send(build_parse_error(parse_err)).await;
            }
        }
    }
    // EOF on stdin (parent died or pipe closed); shut down cleanly.
    drop(out_tx);
    let _ = writer_handle.await;
    0
}

async fn dispatch_with_panic_safety(params: RequestParams) -> Result<serde_json::Value, ServiceError> {
    use std::panic::AssertUnwindSafe;
    use futures::FutureExt;
    let method = params.method_name();
    let result = AssertUnwindSafe(dispatch(params)).catch_unwind().await;
    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(e),
        Err(panic) => Err(ServiceError::Panic {
            method: method.into(),
            message: panic_message(panic),
        }),
    }
}
```

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **Workspace scaffolding.** Add `crates/service-api/` and `crates/service/` to the workspace `Cargo.toml`. Empty crate skeletons. Verify `brokkr check` clean.
2. **`service-api` types + framing layer.** Types as sketched above. `write_message` / `read_message` helpers with frame-size enforcement. JSON-RPC envelope parser with proper error handling. Unit tests: serde round-trip per enum, framing rejects oversize, parser handles malformed JSON gracefully (returns parse-error response with `id=null`).
3. **`service` runtime: `run_service_with_io` + health handler + panic safety.** Generic-IO entry point so tests can drive it via `tokio::io::duplex`. `health.ping` handler returns `HealthPingResponse { version: PROTOCOL_VERSION, pid, uptime_ms }`. Dispatch wrapped in `catch_unwind`. Unit + in-process integration tests for the dispatch loop.
4. **File-based logger.** Service writes to `<app_data>/logs/service.log` with size-based rolling (~10 MB cap, keep 3). Tagged `[service]` prefix. Falls back to stderr if log file can't be opened.
5. **`run_service()` production entry.** Wires real stdin/stdout into `run_service_with_io`. Installs panic hook. Spawns parent-death watcher.
6. **`--service` flag dispatch in `app`.** First thing in `main()`: if args contain `--service`, call `service::run_service()` and exit. Otherwise continue to existing iced boot. Smoke test: `cargo run -p app -- --service` runs the Service in the foreground.
7. **`ServiceClient` core.** Spawn (with platform-gated parent-death setup), bounded stdin/notification channels, pending map (using `serde_json::Value` not untagged enum), typed `request<R>(...)`, `subscribe_notifications`, `Drop` impl that drains the pending map.
8. **Reader + writer + heartbeat tasks.** Reader parses child stdout, routes responses to `pending` and notifications to `notifications_tx` (drops oldest under pressure for low-priority kinds; Phase 1 has none, but the framework is ready). Writer drains `stdin_tx` to the child's stdin. Heartbeat: 30 s, logs missed beats. Reader EOF fails every pending sender with `ClientError::ServiceCrashed`.
9. **Wire `ServiceClient` into `App::boot`.** UI spawns Service; awaits first `health.ping`; asserts `response.version == PROTOCOL_VERSION` (fatal mismatch with clear message). Logs "Service ready (pid=X)". Quit teardown calls `service_client.shutdown()` (request, 30 s timeout, then SIGTERM, then SIGKILL after 5 s).
10. **Race-free parent-death watchers per platform.** Linux: `pre_exec` PR_SET_PDEATHSIG + post-prctl `getppid()` check. macOS: `kqueue` `EVFILT_PROC` + `NOTE_EXIT`. Windows: `OpenProcess` + `WaitForSingleObject`. Manual test on each platform.
11. **Real failure-mode tests** (in-process via `tokio::io::duplex` driving `run_service_with_io`): EOF-during-pending fails callers; malformed JSON returns parse-error and Service stays up; concurrent ping fan-out (100 ids, all distinct, all correlated); version mismatch fails boot; spawn-failure fails boot; panicking handler returns `ServiceError::Panic` and Service stays up; oversize frame rejected without crash.

Total estimated commits: 11. Total estimated LOC: 800-1100 (the framing layer and ClientError handling are real work; previous "600-800" undersold).

## File-by-file changes

**New files:**
- `crates/service-api/Cargo.toml`
- `crates/service-api/src/{lib,request,response,notification,error,framing,version}.rs`
- `crates/service/Cargo.toml`
- `crates/service/src/{lib,dispatch,lifecycle,logging,parent_death}.rs`
- `crates/service/src/handlers/{mod,health,shutdown}.rs`
- `crates/service/tests/{dispatch_in_process,framing,failure_modes}.rs`
- `crates/app/src/service_client.rs`

**Modified files:**
- `Cargo.toml` (workspace) - register the two new crates
- `crates/app/Cargo.toml` - dep on `service-api` and `service`
- `crates/app/src/main.rs` - mode dispatch
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`; teardown calls `service_client.shutdown()` in the window-close path
- `crates/app/src/lib.rs` (or `mod.rs`) - re-export the new module if needed

**Cargo.lock** will change with the new crates plus deps (`dashmap`, `thiserror`, `libc` on Linux, `kqueue` or equivalent on macOS, `winapi`/`windows-sys` on Windows). Committed with the rest per CLAUDE.md.

## Test plan

### Unit tests

- `service-api`: serde round-trip for each `Request` / `Response` / `Notification` variant; framing rejects oversize; JSON-RPC envelope parser handles malformed input.
- `service`: each handler tested in isolation; panic-safe dispatcher catches and converts to `ServiceError::Panic`.
- `service::dispatch`: parser exercised with valid + malformed + oversize lines.

### Integration tests (in-process)

All driven via `tokio::io::duplex` against `run_service_with_io`. No subprocess.

- **`tests/dispatch_in_process.rs`** - happy path: ping → response with correct version/pid/uptime.
- **`tests/framing.rs`** - oversize frame rejected; partial line buffered until newline; multiple frames in one buffered chunk.
- **`tests/failure_modes.rs`**:
  - EOF on the read half: dispatch loop exits cleanly.
  - Malformed JSON line: parse-error response with `id=null`; loop continues.
  - Concurrent fan-out: 100 simultaneous pings; assert 100 distinct ids correlated correctly.
  - Panicking handler (test-only handler that panics): returns `ServiceError::Panic`; Service stays up; subsequent pings succeed.
  - Version mismatch (test-only handler returning a wrong version): client reports `ClientError::VersionMismatch`.

### `ServiceClient` tests

These need a real subprocess to exercise the spawn path. Can wait for Phase 1.5 or land here using `escargot` / `cargo metadata` to locate the built `app` binary.

- Spawn + ping: real subprocess answers `health.ping`.
- Spawn + shutdown: clean exit within timeout.
- SIGKILL the subprocess: pending requests fail with `ClientError::ServiceCrashed`.

### Manual test matrix

- Linux: spawn UI, kill UI with SIGKILL, verify Service exits within 2 s (PR_SET_PDEATHSIG).
- macOS: spawn UI, kill UI with SIGKILL, verify Service exits within polling interval (~2-5 s).
- Windows: spawn UI, kill UI via Task Manager, verify Service exits via `WaitForSingleObject` immediately.
- All: spawn UI, quit normally via the app's quit path, verify Service exits cleanly within 30 s (the shutdown-ack timeout) and no zombie processes remain.
- All: stop the Service externally (`kill <service-pid>`); verify the UI's heartbeat detects it and logs the missed beats. (No respawn yet; that's Phase 1.5.)

## Open questions

These are the questions the planning session should resolve before code starts; flagging them here for the document review.

1. **Single binary vs two binaries.** Default proposal: single binary with `--service` flag. Alternative: ship a separate `ratatoskr-service` binary. Single-binary is simpler operationally; flagging for confirmation. (`--service` stays a public CLI flag; it does not get hidden behind an env var or `--__internal-` convention.)
2. **JSON-RPC library: hand-roll or `jsonrpsee`?** Default proposal: hand-roll for Phase 1 (small surface; framing layer is ~200-300 LOC including all the failure-mode handling). Adopt `jsonrpsee` later if the surface area or batching support justifies it.
3. **`pending` map crate.** `DashMap` for the `id -> oneshot::Sender` table is the path of least resistance. Alternative: `Mutex<HashMap>`. DashMap probably right; minor.
4. **Log tagging mechanism.** Default proposal: a thin wrapper around `log::info!` etc. that prepends `[service]` or `[ui]`. File-based logging via the rolling-file logger; stderr falls back if the log file can't be opened. `tracing` could replace `log` in a future cross-cutting effort.
5. **`ServiceClient` subprocess tests in Phase 1 vs Phase 1.5.** The plan defers real-subprocess tests to Phase 1.5, since in-process `tokio::io::duplex` covers the contract. If we want belt-and-suspenders, `escargot` can spawn the built `app` binary in tests now. Marginal additional value; flagging.

## Verification (end-to-end)

1. `cargo build -p app && ./target/debug/app` (or `cargo run -p app`) - app boots normally, log shows `Service ready (pid=NNN)` within 1 s.
2. `ps -ef | grep ratatoskr` shows two processes: UI (parent) and service (child).
3. Logs show heartbeat round-trips every 30 s with sub-millisecond round-trip times.
4. `<app_data>/logs/service.log` exists and contains the boot + heartbeat lines tagged `[service]`.
5. Quit the app. Service exits cleanly within 30 s via the request/ack handshake. `ps` shows no zombie.
6. Run again, this time SIGKILL the UI process. Service exits within seconds on each platform (Linux: PR_SET_PDEATHSIG; macOS: kqueue NOTE_EXIT; Windows: WaitForSingleObject).
7. Trigger a panic in a test-only handler. Client receives `ServiceError::Panic`; subsequent ping succeeds.
8. Run `cargo test -p service` - integration tests pass.
9. `brokkr check` clean.

## Promotion criteria

This phase is done when:
- All "Exit criteria" in the roadmap's Phase 1 section are satisfied.
- The integration test is green in CI.
- Manual cross-platform shutdown tests pass on Linux, macOS, and Windows (the user runs them).
- Reviewer signoff on this plan + its delivered code.

The next phase (Phase 2 - Action service migration) gets its own equivalent plan document at the time it's tackled.
