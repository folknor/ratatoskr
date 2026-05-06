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

## Phase 6a / 6a-part-2 - write-surface relocation smoke

The 12+ write-surface IPCs added in Phase 6a all have wire-shape round-trip tests + handler unit tests, and the `service_subprocess_*` cohort covers the boundary-crossing path. The items below are the ones whose end-to-end behavior is hard to assert from automation - cold-boot timing, in-flight runner cancellation, file-system-level WAL crash safety. Run on Linux before promoting any phase that touches Service IPC.

### 6. Cold-boot bootstrap snapshots (encryption-key handle end-to-end)

Verifies that `internal.read_bootstrap_snapshots` lands the persisted preferences before the user can interact with them, and that the Service-side decrypt path returns identical UI state to the pre-relocation local-decrypt path.

1. Open Settings, change the theme (e.g. dark), enable Block Remote Images, set Reading Pane to bottom. Wait for the auto-save tick (~1 s) then quit the app cleanly.
2. Relaunch the app.
3. **Expected:** the new theme, remote-image setting, and reading-pane position are visible in the first frame after boot.ready - no flash of defaults persisting beyond the initial paint. The Service log carries an `internal.read_bootstrap_snapshots` dispatch line; the UI log carries `bootstrap snapshots IPC failed` only on error.

If the new theme appears for ~1 s of "default" then snaps to dark: the `Message::BootstrapSnapshotsLoaded` arrival is later than the first paint - acceptable today, but the round-trip should land sub-100 ms in steady-state. Investigate if the gap exceeds ~500 ms.

### 7. Window-close-during-typing draft WAL

Verifies the WAL captures the in-flight keystrokes that the synchronous DB write used to capture pre-relocation, and that the Service drain replays them on next boot before `boot.ready`.

1. Open a compose window, start typing a draft (subject + body). Do not wait for the auto-save tick.
2. Close the compose window via the OS title-bar close (Linux: click the X). The auto-save tick has not fired yet at this point.
3. Confirm `<app_data>/drafts.wal` exists and contains a JSON line for the just-closed draft.
4. Quit the app cleanly.
5. Relaunch the app. The Service boot phase progress should momentarily show "Draining draft WAL..." (BootPhase::DrainingDraftWal).
6. **Expected:** the draft appears in the Drafts folder with the typed content intact. `drafts.wal` is renamed to `drafts.wal.replayed.<epoch_ms>`; no active WAL remains.

If the draft is empty or missing: the WAL append failed (check logs for "Failed to append compose draft to WAL") or the drainer skipped the entry (check logs for "drafts.wal: skipping unparseable line"). If the file is still active after boot: the rename failed - the next boot will idempotently re-replay, but the rename target may need a permission fix.

### 8. Account delete cancels in-flight sync

Verifies `account.delete` cancels per-account runners and completes external-store cleanup inside the single 60 s IPC, replacing the pre-Phase-3 UI-side `cancel_and_await` orchestration.

1. With at least two accounts configured, trigger a manual sync on one of them (sidebar refresh, or wait for the next `SyncTick`). Confirm the sync runner is in flight (Service log: "[sync] account=... starting").
2. While the sync is mid-flight, delete the syncing account from Settings.
3. **Expected:** the deletion completes within ~60 s; the Service log carries `account.delete: sync cancel-and-await(<acct>)` followed by the cleanup report (`N bodies, N inline images, N cache files; search_cleaned=true`). The other account's sync continues unaffected. No orphaned rows in `messages` / `attachments` / `local_drafts` after the delete (verifiable via dev-seed inspector or a SQLite shell).

If the Service log shows the sync runner panicking or the cancel-and-await hanging: the cancellation token plumbed through `SyncRuntime::cancel_account_and_await` is not flipping at a checkpoint the runner observes. Inspect the runner body for missing `cancel_token.is_cancelled()` checks between long-running protocol calls.

### 9. OAuth re-auth via account.update_tokens

Verifies the `account.update_tokens` IPC writes new OAuth tokens onto the existing account row without touching identity or provider columns - the path that replaces the two `with_write_conn` callers in `ui/add_account/{state,oauth}.rs`.

1. Configure a Gmail or Graph (Outlook) account.
2. Forcibly invalidate the access token (e.g. revoke the OAuth grant from the provider's account-management page, or simulate by setting `token_expires_at = 0` via the dev-seed inspector). The next sync attempt fails with an auth error.
3. Trigger the in-app re-auth flow (sidebar prompt or Settings -> Re-authenticate).
4. Complete the OAuth dance in the browser.
5. **Expected:** the Service log carries `account.update_tokens` dispatch; the next sync succeeds. `crates/app/src/ui/add_account/state.rs` and `oauth.rs` no longer touch the writable connection (verifiable via grep).

If the re-auth flow completes in the browser but the next sync still fails: the token persist hit the IPC but landed in the wrong column (provider mismatch in the dynamic-update SET list), or the token-expires-at field was not propagated. Check the Service log for the `account.update_tokens` payload (RedactedString hides the token bytes; the `token_expires_at` and `account_id` are visible).
