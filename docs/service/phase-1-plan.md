# The Service - Phase 1 Plan: Process Boundary Scaffolding

Detailed implementation plan for Phase 1 of `implementation-roadmap.md`. This document is meant to be reviewable end-to-end before any code is written; all subsequent phases get their own equivalent document at the time they're tackled.

## Context

Phase 1 lands the bare scaffolding for the two-process architecture defined in `problem-statement.md`. Specifically: a child process exists, the UI spawns it on start, the two exchange a single JSON-RPC method (`health.ping`), and lifecycle (start, heartbeat, clean shutdown, parent-death detection) all work cleanly. **No real functionality moves across the boundary in this phase.** Sync, the action service, the Tantivy writer, and blob store writes all stay where they are today; the scaffolding just makes the future relocations possible without rewriting infrastructure.

The deliverable is small and well-scoped on purpose: every later phase plugs into this scaffold, so getting the scaffold right is worth more than getting it big.

## Scope

### In scope

1. **Single-binary, dual-mode dispatch.** The existing `ratatoskr` binary gains a `--service` flag. With the flag, it runs the Service entry point and exits when the Service exits. Without, it boots the iced app exactly as today.
2. **Two new workspace crates:**
   - `crates/service-api/` - shared types between UI and Service (JSON-RPC request/response/notification enums, error types, version constant). Pure types crate; no logic.
   - `crates/service/` - the Service runtime: `run_service()` async function that reads JSON-RPC from stdin, dispatches handlers, writes responses to stdout. Phase 1 surface: `health.ping`, `Shutdown` notification.
3. **`ServiceClient` in the app crate.** Spawns the subprocess, manages stdio pipes, correlates request IDs to response futures, runs a background reader for incoming notifications, exposes a typed `request<R, P>(method, params)` API.
4. **Heartbeat.** UI sends `health.ping` every 5 s; logs missed beats. No respawn yet (Phase 8); just visibility.
5. **Parent-death detection.**
   - Linux: `pre_exec` hook calling `prctl(PR_SET_PDEATHSIG, SIGTERM)` so the Service dies if the UI dies.
   - macOS / Windows: a Service-side background task that polls the parent PID every 2 s; exits if the parent is gone.
6. **Clean shutdown.** UI sends a `Shutdown` notification, waits up to 5 s for the Service's stdin to close, escalates to SIGTERM, then SIGKILL after another 5 s.
7. **Logging.** Service writes logs to its stderr, which is inherited by the UI process's stderr (so `cargo run -p app` shows both interleaved). Each log line includes a `[service]` or `[ui]` tag for disambiguation.
8. **Integration test.** A test in `crates/service/tests/spawn_and_ping.rs` that spawns the Service binary in a subprocess, pings it, verifies the response shape, and shuts down cleanly.

### Out of scope

- Any actual functionality moving across the boundary (sync, action service, Tantivy writer, blob store writes). All happens in later phases.
- Respawn-on-crash with backoff. Phase 8.
- Tray icon, autostart, daemon promotion. Out of v1 entirely (Phase 9 optional).
- Schema versioning of the JSON-RPC protocol. Pin format-version-1 in v1; bump method names if the contract changes later.
- Authentication / authorization between UI and Service. They are the same trust domain (UI spawned the Service); stdio is private to the parent-child pair.
- Any production logging story (structured logs, file rotation, etc.). Phase 1 logs to stderr; structured logging is a separate cross-cutting effort.

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

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum RequestParams {
    HealthPing,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ResponseResult {
    HealthPing(HealthPingResponse),
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct HealthPingResponse {
    pub version: u32,
    pub pid: u32,
    pub uptime_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Notification {
    Shutdown,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ServiceError {
    pub code: i32,
    pub message: String,
}
```

A wire-format wrapper type handles the `jsonrpc`/`id`/`method`/`result`/`error` envelope. Pick the simplest path: hand-roll it (~50 LOC of `serde_json::Value` munging) rather than pulling in `jsonrpsee` for one method. We can adopt `jsonrpsee` later if the surface area justifies it.

### `ServiceClient` (UI-side) sketch

In `crates/app/src/service_client.rs`:

```rust
pub struct ServiceClient {
    child: tokio::sync::Mutex<tokio::process::Child>,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<ResponseResult, ServiceError>>>>,
    next_id: AtomicU64,
    notifications_tx: tokio::sync::mpsc::UnboundedSender<Notification>,
}

impl ServiceClient {
    pub async fn spawn() -> Result<Self, SpawnError>;
    pub async fn request(&self, params: RequestParams) -> Result<ResponseResult, ServiceError>;
    pub async fn notify(&self, notification: Notification) -> Result<(), io::Error>;
    pub async fn shutdown(&self) -> Result<(), ShutdownError>;
    pub fn notifications(&self) -> tokio::sync::mpsc::UnboundedReceiver<Notification>;
}
```

Background tasks owned by the `ServiceClient`:
- **Reader task**: parses lines from the child's stdout, dispatches responses to `pending` (by id) or notifications to `notifications_tx` (no id).
- **Heartbeat task**: every 5 s sends `RequestParams::HealthPing`; logs round-trip time; logs warning on missed beat.

### Process spawn

```rust
let child = tokio::process::Command::new(std::env::current_exe()?)
    .arg("--service")
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::inherit())
    .pre_exec_pdeathsig_on_linux()  // platform-gated helper
    .kill_on_drop(true)
    .spawn()?;
```

The `pre_exec_pdeathsig_on_linux` helper wraps `Command::pre_exec` (unsafe) on Linux to call `prctl(PR_SET_PDEATHSIG, SIGTERM)` in the child before exec. On macOS/Windows it's a no-op; the Service-side parent-PID polling covers those platforms.

### Service-side dispatch loop

```rust
pub async fn run_service() -> ! {
    setup_logging();
    spawn_parent_pid_watcher();  // no-op on Linux, polls on mac/win
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        match parse_envelope(&line) {
            Envelope::Request { id, params } => {
                let result = dispatch(params).await;
                let response = build_response(id, result);
                stdout.write_all(serde_json::to_vec(&response)?.as_slice()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
            }
            Envelope::Notification { Notification::Shutdown } => {
                std::process::exit(0);
            }
        }
    }
    // stdin closed (parent died or stdin pipe closed); exit cleanly
    std::process::exit(0);
}
```

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

1. **Workspace scaffolding.** Add `crates/service-api/` and `crates/service/` to the workspace `Cargo.toml`. Empty crate skeletons (lib.rs with a single `pub fn placeholder() {}`). Verify `brokkr check` clean.
2. **`service-api` types.** Define `RequestParams`, `ResponseResult`, `Notification`, `ServiceError`, `PROTOCOL_VERSION`, and the wire envelope wrapper. Unit tests for serde round-trip on each enum.
3. **`service` runtime + health handler.** Implement `run_service()` with stdin reader, dispatch loop, `health.ping` handler. Unit tests for the handler in isolation.
4. **`--service` flag dispatch in `app`.** First thing in `main()`: check args for `--service`, jump to `service::run_service()` and never return. Otherwise continue to existing iced boot. Smoke test: `cargo run -p app -- --service` runs the Service in the foreground; stdin lines get processed.
5. **`ServiceClient` skeleton.** Spawn + lifecycle + Shutdown handshake + `request()` + `notify()`. No reader task yet; verify spawn + shutdown work in isolation.
6. **`ServiceClient` reader task.** Background task that parses child stdout, routes responses + notifications. Add the heartbeat task that sends `health.ping` every 5 s.
7. **Wire `ServiceClient` into `App::boot`.** UI spawns the Service at start; logs "Service ready (pid=X)" on first successful ping. Quit teardown sends `Shutdown`, waits, escalates.
8. **Parent-death detection.** Linux `pre_exec` `PR_SET_PDEATHSIG`; macOS/Windows parent-PID polling task in the Service. Manual test: `kill -9 <ui-pid>` and verify Service exits within a few seconds on each platform.
9. **Integration test.** `crates/service/tests/spawn_and_ping.rs`: spawn the Service binary, ping it, assert response shape, send Shutdown, assert clean exit.
10. **Tagged stderr logger.** `[service] ...` / `[ui] ...` prefixes on all log lines for disambiguation in the interleaved output.

Total estimated commits: 10. Total estimated LOC: 600-800 (mostly types + boilerplate; the actual logic is small).

## File-by-file changes

**New files:**
- `crates/service-api/Cargo.toml`
- `crates/service-api/src/{lib,request,response,notification,error,version}.rs`
- `crates/service/Cargo.toml`
- `crates/service/src/{lib,dispatch,lifecycle,logging}.rs`
- `crates/service/src/handlers/{mod,health}.rs`
- `crates/service/tests/spawn_and_ping.rs`
- `crates/app/src/service_client.rs`

**Modified files:**
- `Cargo.toml` (workspace) - register the two new crates
- `crates/app/Cargo.toml` - dep on `service-api` and `service`
- `crates/app/src/main.rs` - mode dispatch
- `crates/app/src/app.rs` - boot launches `ServiceClient`, stores it on `App`; teardown calls `service_client.shutdown()` in the existing window-close path
- `crates/app/src/lib.rs` (or `mod.rs`) - re-export the new module if needed

**Cargo.lock** will change with the new crates and (probably) `dashmap` or similar for the `pending` map. Committed with the rest per CLAUDE.md.

## Test plan

### Unit tests

- `service-api`: serde round-trip for each `Request` / `Response` / `Notification` variant. Verify envelope parsing handles malformed JSON gracefully.
- `service`: each handler tested in isolation. `health.ping` returns a well-formed `HealthPingResponse`.
- `service::dispatch`: exercise the parser with valid + malformed lines.

### Integration test

`crates/service/tests/spawn_and_ping.rs`:
1. Spawn `env::current_exe()` in service mode (or, in test mode, spawn a Service-binary built specifically for the test - figure out the cleanest pattern).
2. Send a `health.ping` request via the test's stdin pipe to the child.
3. Read the response, assert structure.
4. Send `Shutdown` notification.
5. `child.wait()` returns within 5 seconds with exit code 0.

### Manual test matrix

- Linux: spawn UI, kill UI with SIGKILL, verify Service exits within 2 s (PR_SET_PDEATHSIG).
- macOS: spawn UI, kill UI with SIGKILL, verify Service exits within polling interval (~2-5 s).
- Windows: spawn UI, kill UI via Task Manager, verify Service exits within polling interval.
- All: spawn UI, quit normally via the app's quit path, verify Service exits cleanly within 5 s and no zombie processes remain.
- All: stop the Service externally (`kill <service-pid>`); verify the UI's heartbeat detects it and logs the missed beats. (No respawn yet; that's Phase 8.)

## Open questions

These are the questions the planning session should resolve before code starts; flagging them here for the document review.

1. **Single binary vs two binaries.** Default proposal in this plan is single binary with `--service` flag. Alternative: ship a separate `ratatoskr-service` binary. Single-binary is simpler operationally (no path-lookup, version-drift impossible, smaller distribution surface) but conflates the two builds. I think single-binary is right; flagging for confirmation.
2. **JSON-RPC library: hand-roll or `jsonrpsee`?** Default proposal: hand-roll for Phase 1 (one method, ~50 LOC of envelope handling). Adopt `jsonrpsee` later if the surface area or batching support justifies it. Reasonable to disagree.
3. **Heartbeat interval.** Default proposal: 5 s. Long enough not to be chatty, short enough that a hung Service is noticed within a couple of beats. Tunable later.
4. **`pending` map crate.** `DashMap` for the `id -> oneshot::Sender` table is the path of least resistance; we already have similar use elsewhere. Alternative: `Mutex<HashMap>`. DashMap probably right; minor.
5. **Log tagging mechanism.** Default proposal: a thin wrapper around `log::info!` etc. that prepends `[service]` or `[ui]`. Could also use the `env_logger` formatter or a more structured approach (`tracing`). Phase 1 keeps it simple; revisit if logging gets unwieldy.
6. **What to do if the Service spawn itself fails at app startup.** Probably: log a fatal error and refuse to boot the UI. Once any later phase puts real functionality in the Service, the UI can't usefully operate without it. Flagging for confirmation.

## Verification (end-to-end)

1. `cargo build -p app && ./target/debug/app` (or `cargo run -p app`) - app boots normally, log shows `Service ready (pid=NNN)` within 1 s.
2. `ps -ef | grep ratatoskr` shows two processes: UI (parent) and service (child).
3. Logs show heartbeat round-trips every 5 s, with sub-millisecond round-trip times.
4. Quit the app. Service exits within 5 s. `ps` shows no zombie.
5. Run again, this time SIGKILL the UI process. Service exits within 5 s on Linux (immediately on PR_SET_PDEATHSIG); within polling interval on macOS / Windows.
6. Run `cargo test -p service` - integration test passes.
7. `brokkr check` clean.

## Promotion criteria

This phase is done when:
- All "Exit criteria" in the roadmap's Phase 1 section are satisfied.
- The integration test is green in CI.
- Manual cross-platform shutdown tests pass on Linux, macOS, and Windows (the user runs them).
- Reviewer signoff on this plan + its delivered code.

The next phase (Phase 2 - Action service migration) gets its own equivalent plan document at the time it's tackled.
