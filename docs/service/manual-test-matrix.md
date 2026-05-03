# Service - Manual Test Matrix

Items that must be exercised by hand before any phase that touches Service lifecycle code (Phase 1, Phase 1.5, Phase 8, Phase 9) is considered ready to ship. Each platform's parent-death module carries a `// MANUAL TEST REQUIRED` comment so the matrix doesn't get lost between phases.

The matrix below is the post-Phase-1-test-pass state. Anything once-listed-here-now-automated has a pointer to the test that replaces it.

## Linux

All Linux items are automated as of the Phase 1 test pass. They live in:

- `crates/app/tests/service_subprocess.rs::linux_parent_sigkill_terminates_service_within_two_seconds` - parent SIGKILL fires `PR_SET_PDEATHSIG` and the Service exits within ~2 s.
- `crates/app/tests/service_subprocess.rs::dropping_client_terminates_child_within_one_second` - dropping `ServiceClient` without calling `shutdown()` exits the child within ~1 s; no orphan.
- `crates/app/tests/service_subprocess.rs::println_from_handler_does_not_corrupt_json_rpc_framing` - in-handler `println!` lands in `/dev/null` via the stdio defense; the wire stays well-formed.
- `crates/service/tests/dispatch_in_process.rs::*` - panic safety, in-flight semaphore cap, framing edge cases.

No manual Linux items remain for Phase 1. Re-add only if `brokkr check -p app` stops covering one of the above.

## Windows

The Windows path landed in tree but has not been run on a real Windows host (the implementer was on Linux). Run all three items below before promoting Phase 1.

### 1. Parent-death via Job Object

1. Build the app on Windows: `cargo build -p app --release`.
2. Run it: `cargo run -p app --release`. UI window opens.
3. Confirm two processes in Task Manager: the UI (parent) and one named `ratatoskr` or `app` (the Service child).
4. Kill the UI process via Task Manager's "End task".
5. **Expected:** the Service process disappears from Task Manager immediately (the OS terminates every process in the Job when the parent's Job handle is released).

If the Service stays alive: the Job Object setup in `crates/service/src/parent_death/windows.rs` is wrong. Likely culprits: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` not actually applied, `AssignProcessToJobObject` returning an error that was suppressed, or `OpenProcess(child.id())` failing because of an access-rights mismatch.

### 2. Clean shutdown via the request/ack handshake

1. Run the app on Windows.
2. Quit via the app's normal quit path (window close button, file menu Quit, etc.).
3. **Expected:** within 30 s the Service exits cleanly. A `clean_shutdown` sentinel file appears in the app data dir (`%APPDATA%\org.folknor.ratatoskr\` or wherever `dirs::data_dir()` resolves on Windows). No zombie process in Task Manager.

If the Service is still alive after 30 s: the request/ack timeout escalation is not following through to `TerminateProcess`. Inspect `ServiceClient::shutdown` and the per-method timeout for `RequestParams::Shutdown`.

### 3. Stdio corruption defense via `SetStdHandle(NUL)`

The `test-helpers` feature ships a `test.println` handler that calls `println!` from inside the dispatch loop. Without the stdio defense (`DuplicateHandle` saves the original pipes, `SetStdHandle(STD_OUTPUT_HANDLE, NUL)` redirects the global), that print would land in the JSON-RPC pipe and break framing.

Run:

```
cargo test -p app --release --features test-helpers println_from_handler -- --nocapture
```

**Expected:** the test passes. The `STDIO-CORRUPTION-CANARY-XYZ` string never appears in the JSON-RPC stream that the test reads back; the follow-up `health.ping` after the println still round-trips.

If the test fails on Windows but passes on Linux: the Windows `claim_stdio` in `crates/service/src/stdio_defense.rs` is wrong. Likely culprits: the `SetStdHandle(NUL)` calls returned 0 and the error was missed, or the `DuplicateHandle` of the original pipes lost the inheritability flag the writer end needs.

## Cross-platform smoke checks

These exercise UX behaviors that are too noisy to assert reliably from automation. Run them on Linux and Windows before any phase ship.

### 4. Heartbeat detects an externally-killed Service

1. Run the app.
2. Find the Service PID via `ps -ef | grep ratatoskr` (Linux) or Task Manager (Windows).
3. Kill the Service externally: `kill <service-pid>` on Linux, "End task" in Task Manager on Windows.
4. **Expected:** the UI's log (Service-side log + UI's stderr) shows "service heartbeat exiting" or similar within ~30 s. No respawn; that's Phase 1.5 work.

If the heartbeat task hangs silently: the `ClientError` variant returned by the heartbeat's `request_value_raw` is not being matched correctly in `crates/app/src/service_client.rs::heartbeat_task`.

### 5. SIGTERM to the Service triggers the shutdown drain

Linux only (Windows has no SIGTERM equivalent in this codebase).

1. Run the app.
2. `kill -TERM <service-pid>` (no `-9`).
3. **Expected:** the Service exits within a second; `clean_shutdown` sentinel is written before exit. UI heartbeat then notices the missed beat per item 4.

If the sentinel is missing: the SIGTERM handler in `crates/service/src/sigterm.rs` is not calling `lifecycle.request_shutdown()`, or the dispatch-loop tail drain is being skipped.
