-- description: Production JMAP kick survives forced lag via resident full-reconcile re-push
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- B3b resident lag recovery. The B3a-infra lag-recovery gate drives the
-- test-INJECT path; this gate forces sustained lag against the PRODUCTION
-- JMAP resident kick and asserts the four invariants the spec pins:
--   (a) no livelock - the kick terminates within a bounded wall-clock;
--   (b) no message loss - every message persists after recovery;
--   (c) the cursor never advances past the gap - the lagged drive does not ack
--       before persisting, so the resident full-reconcile re-push refetches
--       from the last durable cursor rather than skipping the dropped events;
--   (d) a clean terminal SyncResult (Completed once it recovers within the
--       resident re-push window).

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_production_lag_backoff")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-lag-backoff@example.test",
    display_name = "Sync JMAP Lag Backoff",
    account_name = "Sync JMAP Lag Backoff",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- Force the production consumer's first drive to report sustained lag. The
-- resident loop must re-subscribe and re-push an Unknown invalidation so the
-- dropped scopes are re-driven rather than silently waiting for unrelated
-- push or poll traffic. The hook is one-shot, so the re-push drives cleanly
-- and the kick completes.
local armed, arm_err = client:request("test.bifrost_arm_hook", {
    account_id = account.account_id,
    hook = { kind = "force_lag" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "force_lag hook was not armed")

harness.clear_mock_requests(admin_endpoint)
local started = harness.now_ms()
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
local elapsed = harness.now_ms() - started
harness.assert(sync_err == nil, "start_sync failed")

-- (a) no livelock: the resident recovery drive must finish well within the
-- start_sync window.
harness.assert(elapsed < 30000, "lagged production kick did not terminate within bounded window")

-- (d) clean terminal: recovery within the bound is a Completed result.
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- (b) no message loss: the dropped events are refetched from the durable
-- cursor after the full-reconcile re-push, so the full fixture persists.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "all messages persist after lag recovery")
harness.assert(state.thread_count >= 1, "thread count")

-- (e) bounded re-drive (§ 4.3 / § 6): the forced lag drove a REAL re-drive
-- through the production resident_consumer_loop's exponential backoff. Assert
-- on the deterministic re-drive TELEMETRY rather than wall-clock timing:
--   * resident_redrive_total >= 1     - the backoff path actually fired;
--   * resident_redrive_total is small - it did NOT hot-loop (the pre-B3c
--     stopgap re-pushed every 250ms forever; the bound caps that);
--   * resident_redrive_attempt == 0   - the clean caught-up edge that
--     recovered the kick reset the backoff, so the next lag re-establishes
--     at the base delay.
local redrive, redrive_err = client:request("test.bifrost_probe", {
    account_id = account.account_id,
    scope = "account",
})
harness.assert(redrive_err == nil, "re-drive telemetry probe failed")
harness.assert(
    redrive.resident_redrive_total ~= nil,
    "resident re-drive telemetry missing (slot not attached?)"
)
harness.assert(
    redrive.resident_redrive_total >= 1,
    "forced lag did not exercise the resident bounded-backoff re-drive"
)
harness.assert(
    redrive.resident_redrive_total <= 5,
    "resident re-drive hot-looped: total=" .. tostring(redrive.resident_redrive_total)
)
harness.assert_eq(
    redrive.resident_redrive_attempt,
    0,
    "clean caught-up edge did not reset the re-drive backoff attempt"
)

harness.write_summary({
    correct = 1,
    elapsed_ms = elapsed,
    message_count = state.message_count,
    thread_count = state.thread_count,
    resident_redrive_total = redrive.resident_redrive_total,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
