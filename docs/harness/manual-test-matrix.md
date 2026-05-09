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

Items 4 and 5 are automated as of the M6 partial pass. They live in:

- `crates/app/tests/service-harness/m6/heartbeat_detects_killed_service.lua` - externally killed Service is detected and recovered by the respawn path.
- `crates/app/tests/service-harness/m6/sigterm_triggers_shutdown_drain.lua` - SIGTERM enters the unrequested drain path, exits, and leaves no `clean_shutdown` sentinel.

No manual cross-platform smoke items remain for heartbeat or SIGTERM.

### 4. Heartbeat detects an externally-killed Service

Automated by `crates/app/tests/service-harness/m6/heartbeat_detects_killed_service.lua`.
The script starts the Service through `spawn_with_events`, sends
`SIGKILL` to the child, and asserts the event stream produces a new
`ChildSpawned` followed by `BootReady` with a different PID.

### 5. SIGTERM to the Service triggers the shutdown drain

Linux only (Windows has no SIGTERM equivalent in this codebase).
Automated by `crates/app/tests/service-harness/m6/sigterm_triggers_shutdown_drain.lua`.

Current contract: external SIGTERM requests shutdown through the
unrequested drain path. The Service exits, but does not write
`clean_shutdown`; that sentinel is reserved for the graceful
request/ack shutdown path.

## Phase 6a / 6a-part-2 - write-surface relocation smoke

The 12+ write-surface IPCs added in Phase 6a all have wire-shape round-trip tests + handler unit tests, and the `service_subprocess_*` cohort covers the boundary-crossing path. Items 6, 7, and 8 are now automated. Item 9 has an automated OAuth persistence slice, with revoked-token sync recovery still pending. Item 10 remains a provider-workflow check until the Graph calendar harness slice lands.

### 6. Cold-boot bootstrap snapshots (encryption-key handle end-to-end)

Automated by `crates/app/tests/service-harness/m6/cold_boot_bootstrap_snapshots.lua`.

The script writes persisted preferences through `settings.set`, shuts
down cleanly, respawns the Service against the same harness data dir,
then asserts `internal.read_bootstrap_snapshots` returns the persisted
theme, reading pane, font size, remote-image, sync-status, and
phishing settings.

### 7. Window-close-during-typing draft WAL

Automated by `crates/app/tests/service-harness/m6/draft_wal_replays_on_boot.lua`.

The script seeds an account, writes `drafts.wal` with one valid draft
entry plus a partial trailing line, boots the Service against the same
data dir, then asserts the draft row was replayed, the active WAL was
rotated, and a `drafts.wal.replayed.*` file exists.

### 8. Account delete cancels in-flight sync

Automated by `crates/app/tests/service-harness/m6/account_delete_cancels_in_flight_sync.lua`.

The script seeds a `harness-slow-sync` account, starts sync, proves a
duplicate start observes the same in-flight run, deletes the account
through `account.delete`, then asserts the sync marker is `cancelled`
and account-scoped rows are gone.

### 9. OAuth re-auth via oauth.exchange_code

Persistence automation lives in `crates/app/tests/service-harness/m6/oauth_reauth_uses_mock_provider.lua`.

Verifies the `oauth.exchange_code` re-auth path writes new OAuth tokens onto the existing account row without touching identity or provider columns - the path that replaces the two `with_write_conn` callers in `ui/add_account/{state,oauth}.rs`.

Remaining automation: run the same flow against an OAuth-enforced sync fixture, start from a revoked/expired token, and assert the follow-up provider sync succeeds with the newly persisted token.

1. Configure a Gmail or Graph (Outlook) account.
2. Forcibly invalidate the access token (e.g. revoke the OAuth grant from the provider's account-management page, or simulate by setting `token_expires_at = 0` via the dev-seed inspector). The next sync attempt fails with an auth error.
3. Trigger the in-app re-auth flow (sidebar prompt or Settings -> Re-authenticate).
4. Complete the OAuth dance in the browser.
5. **Expected:** the Service log carries `oauth.exchange_code` re-auth persistence; the next sync succeeds. `crates/app/src/ui/add_account/state.rs` and `oauth.rs` no longer touch the writable connection (verifiable via grep).

If the re-auth flow completes in the browser but the next sync still fails: the token persist hit the IPC but landed in the wrong column (provider mismatch in the dynamic-update SET list), or the token-expires-at field was not propagated. Check the Service log for the `oauth.exchange_code` re-auth line (RedactedString hides the token bytes; the `account_id` is visible).

### 10. Calendar event create / update / delete via cal_action.execute_plan

Verifies the Phase 6c calendar action pipeline: UI builds a `CalendarActionPlan`, the Service journals as `kind = 'calendar_plan'`, the worker dispatches to `cal_actions::batch_execute`, and the UI awaits the per-plan `CalendarActionCompleted` via `pending_calendar_actions`.

1. Configure at least one calendar account per provider you have credentials for: Google, Graph (Outlook), JMAP (Fastmail), CalDAV.
2. For each provider:
   - Open the calendar editor (double-click an empty slot).
   - Fill title / start / end / location / description.
   - Save.
   - **Expected:** the event appears in the calendar grid within ~2 s (post-`CalendarChanged` debounce). Service log carries `cal_action.execute_plan` -> `cal_actions::batch_execute` -> `CalendarActionCompleted`.
   - Click the event, edit a field, save again. Same expected flow.
   - Delete the event from the popover. Same expected flow; the event disappears from the grid.
3. Provider-failure case (Create only - LocalOnly path):
   - Disconnect the network or revoke the OAuth grant before creating an event.
   - Save a new event.
   - **Expected:** the event appears locally (the UI's editor closes); a Service log line carries `LocalOnly { reason: ... }`. Future Phase: the UI surfaces a "not synced" indicator.

If the event saves but does not appear: the `CalendarChanged` notification was dropped, or the worker's `service_generation` tag mismatched and the UI's per-incarnation drop logic discarded it. Check the Service log for `Notification::CalendarChanged` and `Notification::CalendarActionCompleted` emissions and confirm the awaiter's plan_id matches what the UI handler tracked.

If the event saves locally but never reaches the provider: the dispatcher returned `LocalOnly` but the UI didn't surface it. That's the known gap from 6c-8 (`completion_to_result` collapses everything to `Ok(())` today); Phase 6d revisits.

## Phase 7 - Attachment text extraction + Tantivy indexing

Phase 7 lands the text-extraction pipeline (PDF / OOXML / plain via `ExtractRuntime`), the per-attachment Tantivy doc shape with `match_kind` annotation, the post-boot.ready + hourly `extract.backfill_kick`, the palette command "Rebuild Search Index", and the schema-version-mismatch Wipe rebuild dispatcher. Integration tests for the end-to-end paths are deferred to Phase 8 alongside the rest of the `service_subprocess` flaky-test cohort. Until that lands, the items below cover the user-visible flows.

### 11. Attachment text extraction round trip (cache-miss -> search match annotation)

Verifies the cache-miss enqueue hook in `attachment.fetch`, the `ExtractRuntime` worker, the per-attachment `add_text` doc shape, and the `MatchKind::Attachment` rendering in the search result row.

1. Run the seeded app. The dev-seed corpus must include at least one PDF attachment whose contents you know - if none exists, drop a small known-content PDF into a seeded message via the dev-seed inspector or add it to `dev-seed.toml`.
2. Open the message. The Service log should carry `attachment.fetch` -> cache-miss -> `extract enqueue` for the PDF's `content_hash`.
3. Wait a few seconds. The Service log should carry `extract completed: indexed=N, skipped=N, failed=N` (today the UI logs at `info`; no status-bar surface yet).
4. Open the search bar. Search for a phrase you know is in that PDF (not in the message body or subject).
5. **Expected:** the message appears in the result list. The result row carries a "matched in *<filename>*" annotation under the subject, with a snippet drawn from the extracted PDF text.

If the search does not return the message: extraction failed silently (check the Service log for `extract worker: failed`); the writer never received the `Index` command (check for `WriterCommand::Index` log lines after extraction); or the per-attachment attribution scored body higher than the attachment (acceptable - the body becomes `match_kind` and the attachment lands in `also_matched` if its score crosses the 50% threshold).

If the result appears but with no annotation: `match_kind` is rendering as `Body` (the per-attachment snippet generator scored 0 against the segment). Inspect `crates/app/src/ui/widgets/cards.rs` snippet rendering and `crates/search/src/lib.rs::SearchReadState::search` per-attachment scoring.

### 12. Backfill kick on boot.ready (post-crash recovery)

Verifies the one-shot `extract.backfill_kick` from `Message::ServiceBootReady` catches up after a Service crash mid-extraction.

1. Run the seeded app. Open several PDFs in sequence so multiple `attachment_cache/<content_hash>` files exist with `text_indexed_at IS NULL` initially.
2. Mid-extraction (within a second or two of the first cache-miss), forcibly kill the Service via `kill <service-pid>` (Linux) or "End task" (Windows).
3. Wait for the UI's heartbeat to notice the missed beat (~30 s). The UI shows the Service-respawn cycle.
4. After respawn completes (boot.ready fires), watch the Service log.
5. **Expected:** within a second of `Message::ServiceBootReady` the UI fires `extract.backfill_kick`; the Service log carries `find_unindexed_cached_attachments returned N rows` and N `extract enqueue` lines. Wait for the queue to drain; search the same phrases as item 11. All cached PDFs are now indexed.

If only some attachments backfill: the SELECT `idx_attachments_text_indexed_at` partial index is filtering out rows it shouldn't (verify `cached_at IS NOT NULL AND text_indexed_at IS NULL` matches what's on disk), or the worker's status-aware skip is treating retry-eligible rows as permanent skips.

If nothing backfills: the runtime is not yet installed when the kick fires (race against `spawn_post_ready_extract_startup`); the handler falls through to the defensive no-op. The next hourly tick should catch up, but the boot.ready kick should not be racing.

### 13. Palette "Rebuild Search Index" (Wipe path)

Verifies `CommandId::AppRebuildSearchIndex` -> `client.rebuild_index(Wipe, force=false)` -> `service::rebuild::run_wipe_rebuild` -> `IndexRebuildProgress` / `IndexRebuildCompleted`.

1. Run the seeded app. Confirm search works against a known-indexed phrase (round trip from item 11).
2. Open the command palette (Ctrl+K / Cmd+K) and select "Rebuild Search Index". There is no confirmation modal in v1 - the rebuild starts immediately. (The plan calls a confirmation modal a follow-up; the palette gesture is the gate today.)
3. **Expected:** the Service log carries `WriterCommand::Clear` -> `reset_extracted_text_for_rebuild` -> per-chunk `Index` commands -> `IndexRebuildCompleted`. While the rebuild runs (1-3 seconds on a seeded mailbox; minutes on a real one), search returns no results because the index was cleared. The UI's `Notification::IndexRebuildProgress` arms log progress at `info` level - no status-bar surface yet (Phase 8 carry-forward).
4. After `IndexRebuildCompleted` arrives, **search remains unavailable until the next reader rebind**. v1 ships without the post-completion `SearchReadState::init` reload - the UI keeps the stale reader handle until the next app launch. (Phase 8 carry-forward.) Restart the app to verify the rebuild populated the new index correctly.
5. After restart, search the same phrase from item 11. The result returns with the same "matched in *<filename>*" annotation - confirming the rebuild fired `extract.backfill_kick` at the end and re-extracted attachment text.

If the rebuild never completes: drain step in `run_shutdown_drain` is not running (the rebuild task was orphaned across a Service crash); next boot will not auto-resume - the user has to re-fire the palette command. Acceptable v1 UX; flag for Phase 8 follow-up if it bites.

If `IndexRebuildCompleted` arrives but search still returns 0 results after restart: `extract.backfill_kick` did not fire or the extraction queue did not drain before app exit. Check logs for `extract.backfill_kick` after `IndexRebuildCompleted`.

### 14. Schema-version mismatch triggers Wipe rebuild

Verifies `check_schema_version_and_dispatch` + `spawn_post_ready_schema_rebuild` fire on a `.version` mismatch.

1. Quit the app cleanly. Locate `<app_data>/search_index/.version` (the file holds a single integer).
2. Edit the file to a different value (e.g. `0`). Save.
3. Relaunch the app.
4. **Expected:** the Service log carries `schema_version_mismatch: persisted=0, current=2 -> dispatching Wipe rebuild` (or similar). The post-ready dispatcher fires `handle_rebuild` with `RebuildPolicy::Wipe`; the same chunk-by-chunk progress + completion arrives as item 13. Once `IndexRebuildCompleted` lands, the dispatcher writes the current `INDEX_SCHEMA_VERSION` value back to `.version`. Restart the app once more - boot is a clean no-op (no second rebuild).

If the rebuild fires but `.version` stays at the old value: sentinel-write ordering is broken. The `.version` write must happen *after* `rebuild_in_flight_id` clears - inspect `spawn_post_ready_schema_rebuild`.

If the rebuild does not fire: `check_schema_version_and_dispatch` is not setting `pending_schema_rebuild` on `BootSharedState`, or the post-ready dispatcher is not reading the flag. The user-visible symptom is silent search staleness against the new schema until a manual palette rebuild.

The "search stays live throughout the rebuild" PreserveExisting dual-index path is a Phase 8 carry-forward; v1 shows search-briefly-unavailable instead.
