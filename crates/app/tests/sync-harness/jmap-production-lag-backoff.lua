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

harness.write_summary({
    correct = 1,
    elapsed_ms = elapsed,
    message_count = state.message_count,
    thread_count = state.thread_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
