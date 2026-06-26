-- description: SIGTERM mid-prefetch drains cleanly; respawn recovers PackStore without torn frames
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 120s

-- Phase 7.5 (attachments roadmap) crash-recovery coverage. The audit
-- of PrefetchRuntime + PackStore concluded the shutdown path is
-- sound:
--
--   * `PrefetchRuntime::shutdown` cancels the CancellationToken,
--     aborts in-flight per-item tasks via `JoinSet::abort_all`,
--     awaits the worker.
--   * `PackStore::put` fsyncs the frame bytes BEFORE inserting the
--     index row, so a crash mid-put leaves at worst a wasted disk
--     extent and never an index entry pointing at missing bytes.
--   * `PackStore::open` unconditionally calls `recover_open_pack`
--     which truncates a torn trailing frame and re-indexes
--     fully-written ones via `INSERT OR IGNORE`.
--
-- This script proves all three under load: a slow saehrimnir keeps
-- attachment fetches in flight long enough for SIGTERM to catch one
-- mid-flight, and the post-respawn boot must come up clean.
--
-- Slow saehrimnir's attachment-fetch responses via the existing
-- `POST /test/latency` admin endpoint (Lua binding:
-- `harness.set_latency(endpoint, { per_protocol = { attachment = ms } })`).
-- Default 0 means no delay; we reset to 0 before respawn so the
-- post-recovery fetch can complete promptly.

local LATENCY_MS = 30000

-- We cannot positively confirm "fetch is in flight" from the script:
-- saehrimnir's `mock_requests` log captures JMAP method-call POSTs
-- but not the separate `Blob/get` HTTP GET download endpoint, and
-- `prefetch.progress` only fires at item-finalize time (useless when
-- the item is parked inside the latency sleep). Instead we rely on
-- timing: PrefetchRuntime's worker dispatches within ~1ms of the
-- post-sync sweep enqueue, so a brief sleep after `start_sync`
-- returns puts us reliably inside the worker's HTTP call, parked in
-- saehrimnir's `attachment` latency sleep.
local DISPATCH_GRACE_MS = 500

local function wait_for_prefetch_completed(queue, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "prefetch.completed"
        then
            return notification
        end
    end
    return nil
end

local function wait_until_dead(pid, timeout_ms)
    local deadline = harness.now_ms() + timeout_ms
    while harness.now_ms() < deadline do
        if not harness.pid_is_alive(pid) then
            return true
        end
        harness.sleep(50)
    end
    return not harness.pid_is_alive(pid)
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

-- Arm slow attachment fetches.
harness.set_latency(admin_endpoint, { per_protocol = { attachment = LATENCY_MS } })

local dir = harness.data_dir("sync_sigint_mid_prefetch")

----------------------------------------------------------------------
-- Incarnation A: trigger prefetch, kill mid-fetch.
----------------------------------------------------------------------

local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "spawn A failed")

local ready_a, ready_err_a = client_a:request("BootReady")
harness.assert(ready_err_a == nil, "boot.ready A failed")
harness.assert(ready_a.ready, "boot.ready A returned ready=false")

local queue_a = client_a:notifications()

local account, account_err = client_a:request("TestSeedAccount", {
    email = "sync-sigint-prefetch@example.test",
    display_name = "Sync SIGINT Prefetch",
    account_name = "Sync SIGINT Prefetch",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client_a:start_sync({
    account_id = account.account_id,
}, 60)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- Sync completed; the post-sync sweep has enqueued the attachment
-- onto PrefetchRuntime and the worker has dispatched into the (slow)
-- saehrimnir response. Brief sleep ensures the fetch is genuinely in
-- flight before signalling.
harness.marker("DISPATCH_GRACE")
harness.sleep(DISPATCH_GRACE_MS)

local pid_a = client_a:child_pid()
harness.assert(pid_a ~= nil, "child pid missing")

-- SIGTERM exercises the graceful drain path (request_shutdown ->
-- drain_runtimes -> drain_prefetch). SIGINT also routes through
-- request_shutdown today, so either signal is acceptable; SIGTERM
-- matches the existing m6/sigterm_triggers_shutdown_drain coverage
-- and keeps the assertions aligned.
harness.marker("KILL")
harness.kill(pid_a, "SIGTERM")
harness.assert(wait_until_dead(pid_a, 10000), "Service A did not exit after SIGTERM")

client_a:drop()

----------------------------------------------------------------------
-- Incarnation B: respawn, verify recovery, finish the fetch.
----------------------------------------------------------------------

-- Reset the latency so the post-respawn re-fetch can complete
-- promptly.
harness.set_latency(admin_endpoint, { per_protocol = { attachment = 0 } })

local client_b, err_b = harness.spawn(dir)
harness.assert(err_b == nil, "spawn B failed")

local ready_b, ready_err_b = client_b:request("BootReady")
harness.assert(ready_err_b == nil, "boot.ready B failed (PackStore::open recovery likely failed)")
harness.assert(ready_b.ready, "boot.ready B returned ready=false")

local queue_b = client_b:notifications()

-- Boot recovery kick (dispatch/post_ready.rs) re-enumerates active
-- accounts and re-issues a backfill against attachments with
-- `content_hash IS NULL`. The killed-mid-fetch row stays NULL (the
-- bytes never made it through `PackStore::put`'s fsync-before-INSERT
-- ordering) and will be re-attempted here. If recovery was broken
-- (torn frame not truncated, sealed pack unreadable, etc.), boot
-- would have failed above; here we additionally prove the system can
-- make forward progress after the crash.
local prefetch_done = wait_for_prefetch_completed(queue_b, 30)
harness.assert(prefetch_done ~= nil, "prefetch.completed not observed on respawn")
harness.assert((prefetch_done.fetched or 0) >= 1, "respawn did not re-fetch the killed attachment")
harness.assert_eq(prefetch_done.failed, 0, "respawn prefetch failed count")

-- DB row now has content_hash populated.
local state, state_err = client_b:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState B failed")
local row = nil
for _, attachment in ipairs(state.attachments) do
    if attachment.filename == "sample.txt"
        or attachment.filename == "blob-att-001"
        or attachment.remote_attachment_id == "blob-att-001" then
        row = attachment
        break
    end
end
harness.assert(row ~= nil, "sample attachment missing post-respawn")
harness.assert(row.content_hash ~= nil, "content_hash nil post-respawn")
harness.assert(#row.content_hash == 64, "content_hash wrong width post-respawn")

-- Pack file exists and was either continued from the open pack left
-- by incarnation A (after recover_open_pack truncated any torn tail)
-- or freshly created.
local packs_dir = dir .. "/attachment_packs"
harness.assert(harness.path_exists(packs_dir), "attachment_packs dir missing post-respawn")
harness.assert(
    harness.dir_has_prefix(packs_dir, "data-"),
    "no pack file under attachment_packs/ post-respawn"
)

harness.write_summary({
    correct = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
})

local ok, shutdown_err = client_b:shutdown()
harness.assert(ok, "shutdown B failed")
harness.assert(shutdown_err == nil, "shutdown B returned error")
