-- description: Production IMAP kick survives forced sustained lag with bounded backoff
-- expected: pass
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_production_lag_backoff")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-lag-backoff@example.test",
    display_name = "Sync IMAP Lag Backoff",
    account_name = "Sync IMAP Lag Backoff",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local armed, arm_err = client:request("test.bifrost_arm_hook", {
    account_id = account.account_id,
    hook = { kind = "force_lag" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "force_lag hook was not armed")

local started = harness.now_ms()
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
local elapsed = harness.now_ms() - started
harness.assert(sync_err == nil, "start_sync failed")
harness.assert(elapsed < 30000, "lagged production kick did not terminate within bounded window")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

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
