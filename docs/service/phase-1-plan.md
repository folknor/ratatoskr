# The Service - Phase 1 Plan: Process Boundary Scaffolding

Detailed implementation plan for Phase 1 of `implementation-roadmap.md`. This document is meant to be reviewable end-to-end before any code is written; all subsequent phases get their own equivalent document at the time they're tackled.

## Context

Phase 1 lands the bare scaffolding for the two-process architecture defined in `problem-statement.md`. Specifically: a child process exists, the UI spawns it on start, the two exchange a single JSON-RPC method (`health.ping`), and lifecycle (start, heartbeat, clean shutdown, parent-death detection) all work cleanly. **No real functionality moves across the boundary in this phase.** Sync, the action service, the Tantivy writer, and blob store writes all stay where they are today; the scaffolding just makes the future relocations possible without rewriting infrastructure.

The deliverable is small and well-scoped on purpose: every later phase plugs into this scaffold, so getting the scaffold right is worth more than getting it big.

## Scope

### In scope

1. **Single-binary, dual-mode dispatch.** The existing `ratatoskr` binary gains a `--service` flag. With the flag, it runs the Service entry point and exits. Without, it boots the iced app as today.
2. **Two new workspace crates:**
   - `crates/service-api/` - shared types: `Request`, `Response`, `Notification` enums, `NotificationClass`, `ServiceError`, `PROTOCOL_VERSION`, framing helpers (`write_message`, bounded line decoder). Phase 1 surface: `health.ping` + `shutdown` request.
   - `crates/service/` - runtime. Two entry points:
     - `run_service()` - production entry. Dups the original stdin/stdout to saved FDs, replaces `STDIN_FILENO`/`STDOUT_FILENO` with `/dev/null` (corruption defense), then runs the dispatch loop against the saved FDs.
     - `run_service_with_io<R: AsyncRead, W: AsyncWrite>(stdin, stdout)` - testable entry, generic over IO.
3. **`ServiceClient` in the app crate.** Spawns subprocess, manages stdio pipes, correlates request IDs, exposes typed `request<R, P>(method, params, timeout)`. Background reader task. Background stdout-writer task with bounded queue (the dispatch loop never blocks on stdout). `kill_on_drop` is **disabled**; teardown is explicitly ordered.
4. **Pending-request map.** `DashMap<u64, oneshot::Sender<Result<serde_json::Value, ServiceError>>>` - **not** an untagged response enum. The typed `request()` wrapper deserializes the value into the expected response type after correlating by id. Drop impl drains and rejects every outstanding sender as `ClientError::ServiceCrashed`.
5. **`ServiceClient::Drop` ordering** is specified, not implicit:
   1. Cancel reader / writer / heartbeat task handles via `JoinHandle::abort()`.
   2. Await tasks with a short deadline (200 ms).
   3. Close stdin (Service sees EOF on the read half, exits cleanly).
   4. Wait briefly for child exit.
   5. SIGKILL only if the child is still alive after the wait.
   6. Drain pending map; reject every outstanding sender.
6. **Per-method timeout policy.** Declared at the API definition site, not at call sites. Phase 1 table:
   | Method | Timeout |
   |--------|---------|
   | `health.ping` | 5 s |
   | `Shutdown` | 30 s |

   Expired requests evict their pending entry.
7. **Notification class taxonomy.** `enum NotificationClass { Coalesce, Drop, MustDeliver }` per-method, declared in `service-api`. Phase 1 has no notifications, but the type lands so Phase 2's first notifications classify cleanly:
   - `Coalesce { key }`: latest-wins on the enqueue side. For `sync.progress`, `index.progress`.
   - `Drop`: drop oldest under queue pressure. For advisory events.
   - `MustDeliver`: never coalesced or dropped. For state changes (`action.completed`, `index.committed`, `push.event`).
8. **Single ordered notification channel.** One bounded `mpsc` channel (cap 1024) carries all notifications in wire order. Per-class enqueue policy: `Coalesce` overwrites by key; `Drop` evicts oldest on full; `MustDeliver` uses awaited `send` so backpressure flows back through the OS pipe buffers to the producing handler. (See problem-statement.md "Single ordered notification channel at the UI client" for why this beats the two-channel design.)
9. **Reader task pipeline.** Reader parses lines, dispatches responses to the pending map (separate from notifications), and enqueues notifications via the per-class policy above. A slow UI consumer of notifications stalls Service writes (correct for `MustDeliver`) but never stalls response delivery, since responses go through the pending map directly.
10. **Bounded in-flight handlers.** Service-side dispatch holds at most N (default 64) concurrent handlers via a semaphore; further requests wait rather than ballooning Service memory under a pathological client. **Permit acquired *inside* the spawned handler task**, not in the dispatch loop, so the dispatch loop keeps reading stdin even while 64 slow handlers are in flight. Heartbeat handler bypasses the semaphore.
11. **Inbound frame cap (4 MiB) enforced *during* read**, not after. Use a bounded line decoder (`tokio_util::codec::LinesCodec::new_with_max_length(MAX_FRAME_BYTES)` or equivalent `read_until` against a `Take`-wrapped reader). A 1 GiB no-newline payload must not OOM the Service before the cap fires. The Phase 1 self-contradiction "uncapped in v1" goes away.
12. **Heartbeat.** UI sends `health.ping` every 30 s; logs round-trip + missed beats. No respawn (Phase 1.5). Heartbeat handler bypasses the in-flight semaphore so heavy load can't starve it.
13. **Parent-death detection (v1: Linux + Windows; macOS deferred to post-1.0, design retained in `problem-statement.md`).** Linux closes the parent-died-before-registration race with registration plus a "is the parent still alive?" recheck; Windows avoids the race via Job Object semantics.
    - Linux: `pre_exec` hook calling `prctl(PR_SET_PDEATHSIG, SIGTERM)` + post-prctl `getppid() == 1` check at startup.
    - Windows: parent creates a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns the Service to it before spawning. No PID lookup, no PID-reuse race. (`OpenProcess(parent_pid)` after startup is rejected: TOCTOU between exec and OpenProcess on Windows is a real bug class.)
14. **Clean shutdown.** `shutdown` is a **request**. UI awaits the response with a 30 s timeout, then SIGTERM (no-op on Windows; `start_kill` is `TerminateProcess`-equivalent), then SIGKILL after another 5 s. Service-side SIGTERM handler triggers the same shutdown drain (flush Tantivy, close pack files, write clean-shutdown sentinel) as the request-driven path. No torn writes via the SIGTERM path.
15. **Version handshake.** First `health.ping` after spawn asserts `response.version == PROTOCOL_VERSION`; mismatch is fatal boot error with a clear "binary mismatch" message. The `health.ping` request envelope shape is **frozen** for v1 - any future Service binary must still parse and respond to a v1 ping (otherwise the handshake catches mismatches only when responses can be parsed at all).
16. **Panic safety.** Every handler runs inside `AssertUnwindSafe(...).catch_unwind()`. Panics return `ServiceError::Panic { method, message }`; the dispatch loop continues. A panicking PDF extractor in Phase 7 won't kill the Service. **The process-level panic hook writes to the Service log file** before the default behavior runs - otherwise panics in non-handler tasks (e.g. tokio runtime worker threads) vanish in production windowed UI.
17. **Notification dispatch into iced.** Subscription recipe (mpsc receiver wrapped per the existing `JmapPushReceiver` pattern in `crates/app/src/handlers/provider.rs`). Phase 1 emits no notifications, but the recipe lands so Phase 2 plugs in cleanly the next day.
18. **File-based logging.** Service writes to `<app_data>/logs/service.<pid>.log` with simple size-based rolling (~10 MB cap, keep 3). PID in the filename avoids the multi-writer race during respawn. A `service.log` symlink in the same directory points at the current Service. stderr stays for `cargo run` debugging. Tagged `[service]` / `[ui]` prefixes for disambiguation in the interleaved console output.
19. **Sensitive-value logging policy** (defined in `problem-statement.md` § IPC). Loggable: method names, request IDs, account IDs, timing. Not loggable: any params/results payload contents, OAuth auth codes, message bodies, search queries, draft content. Wire types use `RedactedString` / `RedactedBytes` wrappers; framing layer's logging hook records method + id + timing only.
20. **Integration tests via in-process dispatch** (`tokio::io::duplex` driving `run_service_with_io`) cover the dispatch contract.
21. **Real-subprocess smoke tests** land in Phase 1 too. In-process duplex by definition cannot test: `--service` flag dispatch, real stdio pipe wiring, child cleanup on Drop, version mismatch from a binary that genuinely runs `--service` mode of itself, OS pipe buffering, parent-death detection. Use `escargot` or workspace-built helper. Minimum set: spawn + ping; spawn + clean shutdown; spawn + drop without shutdown (no orphan); Linux SIGKILL of UI -> Service exits within 2 s. Windows parent-death stays manual.
22. **Real failure-mode tests** (in-process duplex). EOF-during-pending-request, malformed JSON, concurrent ping fan-out (id correlation), version mismatch, spawn failure, panicking handler, oversize frame rejection without OOM.

### Out of scope

- Any actual functionality moving across the boundary.
- Respawn-on-crash. Phase 1.5.
- Tray icon, autostart, daemon promotion.
- Schema versioning of the JSON-RPC protocol. Pin format-version-1 in v1; bump method names if the contract changes later.
- Authentication / authorization between UI and Service. Same trust domain.
- Schema migrations + encryption-key relocation - those are Phase 1.5, not Phase 1.
- Single-instance file lock - those are Phase 1.5, not Phase 1. The lock guards against concurrent schema migrations, which only land in Phase 1.5.

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
Svc:   {"jsonrpc":"2.0","method":"sync.progress","params":{"account_id":"...","current":42,"total":1000}}\n
```

(No `id` field on notifications, per JSON-RPC 2.0 spec. Phase 1 emits no notifications - this example is illustrative for the framing. **`shutdown` is a request, not a notification** - see the shutdown handshake section.)

**Framing:** newline-delimited JSON. Reader uses a bounded line decoder (`tokio_util::codec::LinesCodec::new_with_max_length(MAX_FRAME_BYTES)` or `read_until` against a `Take`-wrapped reader) so the cap enforces *during* read - a 1 GiB no-newline payload must not OOM the Service before the check fires. Per-line size is capped at 4 MiB; oversize frames are rejected at the framing layer with a parse-error response.

### Type definitions (sketch; final shape settles in code)

In `service-api/src/lib.rs`:

```rust
pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;  // 4 MiB

// Method names use dotted form (e.g. "health.ping"); explicit per-variant
// rename strings produce that on the wire. snake_case derive would
// produce "health_ping" instead.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum RequestParams {
    #[serde(rename = "health.ping")]
    HealthPing,
    #[serde(rename = "shutdown")]
    Shutdown,        // request, not notification - we await an ack
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HealthPingResponse {
    pub version: u32,
    pub pid: u32,
    pub uptime_ms: u64,
}

// `health.ping` envelope is FROZEN for v1 - any future Service binary
// must still parse and respond to this exact shape, even if other methods
// change incompatibly. This keeps version-mismatch detection working when
// the rest of the protocol has rotated.

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ShutdownResponse {
    pub flushed_ok: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Notification {
    // empty in Phase 1 - the framework lands but nothing emits notifications yet.
    // Each notification declares its NotificationClass via a const fn
    // method_class() so the dispatcher can route on it.
}

/// How a notification behaves under queue pressure.
pub enum NotificationClass {
    /// Latest-wins coalescing on the enqueue side, keyed by `key`.
    Coalesce { key: CoalesceKey },
    /// Drop oldest under pressure. Advisory events.
    Drop,
    /// Never coalesced or dropped. Backpressure on producer.
    /// State changes: action.completed, index.committed, push.event.
    MustDeliver,
}

/// Sensitive payloads use these wrappers so a stray Debug print or
/// log call site cannot leak content into <app_data>/logs/service.*.log.
pub struct RedactedString(String);
pub struct RedactedBytes(Vec<u8>);
// Debug impl prints `<redacted len=N>`, not the content.
// Serialize/Deserialize behave normally.

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
    #[error("another instance is already running")]
    AnotherInstanceRunning,
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

### `ServiceClient` (UI-side) sketch

In `crates/app/src/service_client.rs`:

```rust
pub struct ServiceClient {
    child: tokio::sync::Mutex<tokio::process::Child>,
    stdin_tx: tokio::sync::mpsc::Sender<Vec<u8>>,  // bounded channel into the writer task
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, ServiceError>>>>,
    next_id: AtomicU64,
    // Single ordered notification channel. Cross-class FIFO is preserved by
    // having ONE channel; per-class policy (Coalesce/Drop/MustDeliver) is
    // enforced on enqueue. See problem-statement.md "Single ordered notification
    // channel at the UI client" for the rationale.
    notif_tx: tokio::sync::mpsc::Sender<Notification>,    // cap 1024
    reader_handle: tokio::task::JoinHandle<()>,
    writer_handle: tokio::task::JoinHandle<()>,
    heartbeat_handle: tokio::task::JoinHandle<()>,
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

    // Per-method timeouts come from the API definition; callers don't pass them.
    pub async fn request<R: DeserializeOwned>(
        &self,
        params: RequestParams,
    ) -> Result<R, ClientError>;

    pub async fn shutdown(&self) -> Result<(), ClientError>;

    /// Subscribe to notifications. Returns the receiver for the single
    /// ordered notification channel. Cross-class FIFO is preserved.
    /// The recipe wraps this in an iced subscription, mirroring the
    /// JmapPushReceiver pattern in crates/app/src/handlers/provider.rs
    /// so Phase 2 plugs in cleanly.
    pub fn subscribe_notifications(&self) -> tokio::sync::mpsc::Receiver<Notification>;
}

impl Drop for ServiceClient {
    fn drop(&mut self) {
        // Ordered teardown - kill_on_drop is DISABLED on the Child handle
        // so this Drop is the only thing terminating the subprocess.
        //
        //  1. Cancel reader / writer / heartbeat tasks.
        self.reader_handle.abort();
        self.writer_handle.abort();
        self.heartbeat_handle.abort();
        //  2. (Awaiting handles isn't possible in sync Drop. The abort()
        //     calls are sufficient; the tokio runtime cleans up after.)
        //  3. Close stdin so the Service sees EOF on the read half.
        //     This is the polite shutdown signal.
        //  4. Wait briefly for child exit (try_wait, not blocking).
        //  5. SIGKILL the child if it's still alive.
        //  6. Drain pending; reject every outstanding sender with ServiceCrashed.
        for (_, sender) in std::mem::take(&mut *self.pending.deref()) {
            let _ = sender.send(Err(ServiceError::Internal("client dropped".into())));
        }
    }
}
```

Background tasks owned by the `ServiceClient`:
- **Reader task**: parses lines from the child's stdout (bounded line decoder enforcing `MAX_FRAME_BYTES` during read), dispatches responses to `pending` (by id) and notifications to the single ordered `notif_tx`. Per-class enqueue policy: `Coalesce` finds and overwrites the existing entry for the same key (no allocation if found, append otherwise); `Drop` evicts oldest on full; `MustDeliver` uses awaited `send` so OS pipe buffers fill and Service-side writes backpressure naturally. Responses go to the pending map, separate from notifications, so a slow UI consumer of notifications cannot stall response delivery to the requesting code path. On EOF, fails every pending sender with `ClientError::ServiceCrashed`.
- **Writer task**: drains `stdin_tx` and writes to the child's stdin. Bounded; if the child can't keep up the channel applies backpressure to callers.
- **Heartbeat task**: every 30 s sends `RequestParams::HealthPing`; logs round-trip time; logs warning on missed beat. Exits when `stdin_tx` send fails (Service died). No respawn until Phase 1.5.

### Process spawn

```rust
let mut cmd = tokio::process::Command::new(std::env::current_exe()?);
cmd.arg("--service")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::inherit())
    .kill_on_drop(false);   // Disabled - explicit ordered teardown in Drop.

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

#[cfg(target_os = "windows")]
{
    // Create a Job Object with KILL_ON_JOB_CLOSE BEFORE spawning the child.
    // When the UI's handle to the Job is released (UI dies), the OS terminates
    // every process in the Job - no PID lookup, no PID-reuse race. Side benefit:
    // grandchildren the Service spawns later (PDF/OOXML extractors in Phase 7)
    // also get killed when the parent dies.
    let job = create_job_with_kill_on_close()?;
    let child = cmd.spawn()?;
    assign_process_to_job(&job, child.id())?;
    // Hold `job` for the lifetime of ServiceClient.
}

#[cfg(not(target_os = "windows"))]
let child = cmd.spawn()?;
```

### Service-side dispatch loop

```rust
pub async fn run_service() -> ! {
    setup_logging();          // <app_data>/logs/service.<pid>.log + symlink
    install_panic_hook();     // writes to log file before default behavior
    setup_sigterm_handler();  // triggers shutdown drain on SIGTERM (Unix)
    spawn_parent_death_watcher();   // platform-gated; race-free per platform
    // Single-instance lock arrives in Phase 1.5 (gates concurrent schema migrations).

    // Stdio corruption defense. Dup the original stdin/stdout to saved FDs;
    // replace STDIN_FILENO / STDOUT_FILENO with /dev/null. Any transitive
    // println!, default tracing-subscriber stdout, panic-handler stdin read,
    // etc. now lands harmlessly on /dev/null instead of desynchronizing
    // the JSON-RPC framing.
    let (stdin_fd, stdout_fd) = dup_stdio_to_saved_fds();
    redirect_real_stdio_to_devnull();
    let stdin = tokio::io::stdin_from_fd(stdin_fd);
    let stdout = tokio::io::stdout_from_fd(stdout_fd);

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

    // Bounded in-flight handler semaphore (default 64). Heartbeat handler
    // bypasses this; everything else queues here so a pathological client
    // can't balloon Service memory.
    let inflight = Arc::new(tokio::sync::Semaphore::new(64));

    // Bounded line decoder: enforces MAX_FRAME_BYTES *during* read so a
    // 1 GiB no-newline payload never gets fully buffered.
    let mut lines = bounded_line_stream(reader, MAX_FRAME_BYTES);
    while let Some(line) = lines.next().await {
        let line = match line {
            Ok(line) => line,
            Err(LineError::Oversize) => {
                log_oversize_frame();
                let _ = out_tx.send(build_parse_error("frame too large")).await;
                continue;
            }
            Err(LineError::Io(e)) => break,  // pipe closed / parent died
        };
        match parse_envelope(&line) {
            Ok(Envelope::Request { id, params }) => {
                // Acquire the in-flight permit *inside* the spawned task,
                // not here. Acquiring before tokio::spawn would block the
                // dispatch loop's stdin read whenever 64 slow handlers
                // are in flight, queuing fast methods behind slow ones.
                let inflight = inflight.clone();
                let out_tx = out_tx.clone();
                tokio::spawn(async move {
                    let _permit = match params.bypasses_semaphore() {
                        true => None,
                        false => Some(inflight.acquire_owned().await.unwrap()),
                    };
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
    run_shutdown_drain().await;  // flush Tantivy, close pack files, write sentinel
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
2. **`service-api` types + bounded framing layer.** `RequestParams` (with explicit `#[serde(rename = "...")]` for dotted method names), `HealthPingResponse`, `ShutdownResponse`, `Notification`, `NotificationClass`, `RedactedString` / `RedactedBytes`, `ServiceError`, `PROTOCOL_VERSION`, `MAX_FRAME_BYTES`. Bounded line decoder enforcing `MAX_FRAME_BYTES` *during* read. `write_message` helper with compact-only serialization. JSON-RPC envelope parser. Unit tests: serde round-trip, framing rejects oversize without buffering whole line, malformed JSON returns parse-error.
3. **`service` runtime: `run_service_with_io` + health handler + panic safety + bounded in-flight semaphore.** Generic-IO entry point. `health.ping` handler returns `HealthPingResponse { version: PROTOCOL_VERSION, pid, uptime_ms }`. Dispatch wrapped in `catch_unwind`. In-flight handler semaphore (cap 64; heartbeat bypasses). Unit + in-process integration tests for the dispatch loop.
4. **File-based logger + sensitive-value redaction.** Service writes to `<app_data>/logs/service.<pid>.log` with size-based rolling (~10 MB cap, keep 3); maintains a `service.log` symlink to the current file. Tagged `[service]` prefix. Falls back to stderr if log file can't be opened. Verify `Debug` impls for `RedactedString` / `RedactedBytes` print `<redacted len=N>` form.
5. **`run_service()` production entry.** Wires real stdin/stdout into `run_service_with_io`. Stdio dup-and-replace defense (Linux: dup `STDIN_FILENO`/`STDOUT_FILENO` to saved FDs, replace originals with `/dev/null`; Windows: equivalent via `DuplicateHandle` + `SetStdHandle` against `NUL`). Installs panic hook (writes to log file before default behavior). Sets up SIGTERM handler that triggers the shutdown drain. (Single-instance file lock lands in Phase 1.5 alongside schema migrations, not here.)
6. **Race-free parent-death watchers (v1: Linux + Windows).** Two commits, one per platform, since each is a self-contained module:
   - 6a (Linux): `pre_exec` PR_SET_PDEATHSIG + post-prctl `getppid() == 1` check.
   - 6b (Windows): `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` Job Object setup. Lives on the spawn (UI) side; included here to keep the platform parent-death modules together. Manual test on each platform.
7. **`--service` flag dispatch in `app`.** First thing in `main()`: if args contain `--service`, call `service::run_service()` and exit. Otherwise continue to existing iced boot. Smoke test: `cargo run -p app -- --service` runs the Service in the foreground.
8. **`ServiceClient` core.** Spawn with `kill_on_drop(false)`, Job Object on Windows. Bounded stdin / notification channels (separate `MustDeliver` and `Coalesce`/`Drop` lanes). Pending map (using `serde_json::Value` not untagged enum). Typed `request<R>(...)` with timeouts pulled from per-method declaration. `subscribe_notifications`. `ClientError` enum (including `AnotherInstanceRunning`).
9. **`ServiceClient::Drop` ordering.** Cancel reader/writer/heartbeat handles; close stdin; brief wait; SIGKILL if still alive; drain pending. Documented as a contract. Test: drop a `ServiceClient` without calling `shutdown()`; child exits within 1 s; no orphan.
10. **Reader + writer + heartbeat tasks.** Reader parses child stdout via the bounded line decoder; routes responses to `pending` and notifications via `try_send` to the appropriate lane (Coalesce/Drop = lossy, MustDeliver = strict). Writer drains `stdin_tx` to the child's stdin. Heartbeat: 30 s, logs missed beats. Reader EOF fails every pending sender with `ClientError::ServiceCrashed`. Heartbeat task exits when stdin send fails.
11. **Notification dispatch into iced.** `Subscription::run` recipe wrapping the `subscribe_notifications` mpsc receiver - mirrors `crates/app/src/handlers/provider.rs`'s `JmapPushReceiver` pattern. Phase 1 has no notifications, but the recipe lands so Phase 2 plugs in the next day.
12. **Wire `ServiceClient` into `App::boot`.** UI spawns Service; awaits first `health.ping` (5 s timeout in Phase 1; extended in Phase 1.5); asserts `response.version == PROTOCOL_VERSION` (fatal mismatch with clear message). Logs "Service ready (pid=X)". Quit teardown calls `service_client.shutdown()` (request, 30 s timeout, then SIGTERM, then SIGKILL after 5 s, then drop).
13. **Real failure-mode tests** (in-process via `tokio::io::duplex` driving `run_service_with_io`): EOF-during-pending fails callers; malformed JSON returns parse-error and Service stays up; concurrent ping fan-out (100 ids, all distinct, all correlated); version mismatch fails boot; spawn-failure fails boot; panicking handler returns `ServiceError::Panic` and Service stays up; oversize frame rejected without buffering and without crash; bounded in-flight semaphore caps concurrent handlers.
14. **Real-subprocess smoke tests.** Use `escargot` or workspace-built helper to locate the `app` binary. Tests:
    - Spawn + `health.ping` round-trip succeeds against a real subprocess.
    - Spawn + `Shutdown` request returns ack; child exits within 30 s.
    - Spawn + drop `ServiceClient` without shutdown; child exits within a few seconds; no orphan.
    - (Linux only) Spawn + SIGKILL the parent of the test harness; subprocess exits within 2 s. Windows parent-death stays manual.

## File-by-file changes

**New files:**
- `crates/service-api/Cargo.toml`
- `crates/service-api/src/{lib,request,response,notification,error,framing,version,redacted}.rs`
- `crates/service/Cargo.toml`
- `crates/service/src/{lib,dispatch,lifecycle,logging,sigterm,stdio_defense}.rs` (Phase 1.5 adds `instance_lock.rs` with the schema migration work.)
- `crates/service/src/parent_death/{mod,linux,windows}.rs`
- `crates/service/src/handlers/{mod,health,shutdown}.rs`
- `crates/service/tests/{dispatch_in_process,framing,failure_modes}.rs`
- `crates/service/tests/subprocess_smoke.rs` - real-subprocess tests via `escargot`.
- `crates/app/src/service_client.rs`
- `crates/app/src/service_subscription.rs` - iced subscription recipe wrapping `subscribe_notifications`.

**Modified files:**
- `Cargo.toml` (workspace) - register the two new crates
- `crates/app/Cargo.toml` - dep on `service-api` and `service`
- `crates/app/src/main.rs` - mode dispatch
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`; teardown calls `service_client.shutdown()` in the window-close path
- `crates/app/src/lib.rs` (or `mod.rs`) - re-export the new module if needed

**Cargo.lock** will change with the new crates plus deps (`dashmap`, `thiserror`, `libc` on Linux, `winapi`/`windows-sys` on Windows, plus a cross-platform file-lock crate - candidates: `fs2`, `fd-lock`). Committed with the rest per CLAUDE.md.

## Test plan

### Unit tests

- `service-api`: serde round-trip for each `Request` / `Response` / `Notification` variant; bounded line decoder rejects oversize *during* read (assert peak buffer size stays bounded); JSON-RPC envelope parser handles malformed input; `RedactedString` / `RedactedBytes` Debug never reveals content.
- `service`: each handler tested in isolation; panic-safe dispatcher catches and converts to `ServiceError::Panic`; in-flight semaphore caps concurrent handlers.
- `service::dispatch`: parser exercised with valid + malformed + oversize lines.

### Integration tests (in-process)

All driven via `tokio::io::duplex` against `run_service_with_io`. No subprocess.

- **`tests/dispatch_in_process.rs`** - happy path: ping -> response with correct version/pid/uptime.
- **`tests/framing.rs`** - oversize frame rejected without buffering whole line; partial line buffered until newline; multiple frames in one buffered chunk.
- **`tests/failure_modes.rs`**:
  - EOF on the read half: dispatch loop exits cleanly after running the shutdown drain.
  - Malformed JSON line: parse-error response with `id=null`; loop continues.
  - Concurrent fan-out: 100 simultaneous pings; assert 100 distinct ids correlated correctly.
  - Panicking handler (test-only handler that panics): returns `ServiceError::Panic`; Service stays up; subsequent pings succeed.
  - Version mismatch (test-only handler returning a wrong version): client reports `ClientError::VersionMismatch`.
  - In-flight semaphore: 200 concurrent slow handlers; assert at most 64 run at once; queued handlers complete eventually; heartbeat handler bypasses the cap.
  - Test-only handlers (panic-injecting, version-mismatch) are `#[cfg(test)]`-gated so they cannot ship.

### Real-subprocess smoke tests (`tests/subprocess_smoke.rs`)

Land in Phase 1, using `escargot` to locate the built `app` binary. Cover what in-process duplex by definition cannot:

- **Spawn + ping** - real subprocess answers `health.ping`.
- **Spawn + shutdown** - clean exit within timeout, sentinel file written.
- **Spawn + drop without shutdown** - dropping `ServiceClient` (Drop ordering) terminates child within 1 s; no orphan process.
- **Linux only: SIGKILL the parent process** - subprocess exits within 2 s via PR_SET_PDEATHSIG. (Run with a wrapper test harness that forks; Windows parent-death stays manual.)
- **Spawn-failure** - point `current_exe` at a non-existent binary; assert clear error, no hang.
- **Stdout corruption defense** - test-only build path that calls `println!` from a transitive dep; assert the JSON-RPC framing on the saved-FD stdout is unaffected.

### Manual test matrix (run before each phase ships)

The full matrix lives in [`manual-test-matrix.md`](manual-test-matrix.md). Linux items are now automated; Windows items 1 - 3 (parent-death via Job Object, clean shutdown via the request/ack handshake, stdio corruption defense) are still manual and must run on a real Windows host before Phase 1 can be promoted. The matrix gets re-run at the close of every phase that touches lifecycle code (Phases 1, 1.5, 8, 9). Each platform's parent-death module carries a `// MANUAL TEST REQUIRED` comment so the matrix doesn't get lost.

## Open questions

Settled before this plan landed:

- **Single binary with `--service` flag** (not two binaries; not env-var hidden). Decided in problem-statement review.
- **Real-subprocess tests land in Phase 1**, not Phase 1.5. In-process duplex doesn't cover spawn / pipe wiring / Drop / parent-death; the marginal cost of `escargot`-driven smoke tests is small.

Resolve in implementation:

1. **JSON-RPC library: hand-roll or `jsonrpsee`?** Default: hand-roll for Phase 1 (small surface). Adopt `jsonrpsee` later if surface area or batching justifies it.
2. **`pending` map crate.** `DashMap` for `id -> oneshot::Sender` is the path of least resistance. Alternative: `Mutex<HashMap>`. DashMap probably right; minor.
3. **Log tagging mechanism.** Default: thin wrapper around `log::info!` etc. that prepends `[service]` or `[ui]`. File-based logging via rolling-file logger; stderr falls back if the log file can't be opened. `tracing` could replace `log` in a future cross-cutting effort.
4. **Bounded line decoder choice.** `tokio_util::codec::LinesCodec::new_with_max_length` vs. hand-rolled `read_until` against `Take`. Both work; LinesCodec is well-trodden but pulls another crate. Decide in implementation; not a design-level question.
5. **Bidirectional notifications.** Phase 1 only declares Service -> UI notifications. Should `service-api` also accommodate UI -> Service notifications (e.g. "user opened thread X" for prefetch)? Not needed in Phase 1 but the dispatch loop shape is decided here. Default: requests-only from UI; if a future phase needs UI notifications, add at that time. Flagging so the API isn't accidentally one-directional.

## Verification (end-to-end)

1. `cargo build -p app && ./target/debug/app` (or `cargo run -p app`) - app boots normally, log shows `Service ready (pid=NNN)` within 1 s.
2. `ps -ef | grep ratatoskr` shows two processes: UI (parent) and service (child).
3. Logs show heartbeat round-trips every 30 s with sub-millisecond round-trip times.
4. `<app_data>/logs/service.<pid>.log` exists and contains the boot + heartbeat lines tagged `[service]`. Symlink `service.log` points at the current Service. No payload contents in the log.
5. Quit the app. Service exits cleanly within 30 s via the request/ack handshake; clean-shutdown sentinel file present. `ps` shows no zombie.
6. Run again, this time SIGKILL the UI process. Service exits within seconds on v1 platforms (Linux: PR_SET_PDEATHSIG + getppid recheck; Windows: Job Object KILL_ON_JOB_CLOSE).
7. Run a third time, SIGTERM the Service externally (`kill <service-pid>`). Service runs the shutdown drain (sentinel written) before exit.
8. Trigger a panic in a test-only handler. Client receives `ServiceError::Panic`; subsequent ping succeeds.
9. Run `cargo test -p service` - all in-process and real-subprocess tests pass.
10. Send an oversize frame (>4 MiB no-newline payload) to the Service. Memory usage stays bounded; parse-error response received; Service stays up.
11. `brokkr check` clean.

(Two-instance test moves to the Phase 1.5 verification list, when the file lock lands.)

## Promotion criteria

This phase is done when:
- All "Exit criteria" in the roadmap's Phase 1 section are satisfied.
- The integration test is green in CI.
- Manual cross-platform shutdown tests pass on Linux and Windows (the user runs them).
- Reviewer signoff on this plan + its delivered code.

The next phase (Phase 2 - Action service migration) gets its own equivalent plan document at the time it's tackled.
