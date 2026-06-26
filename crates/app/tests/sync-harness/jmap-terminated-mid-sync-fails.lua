-- description: JMAP resident sync reports a structured failure when the consumer terminates mid-sync
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s

local dir = harness.data_dir("sync_jmap_terminated_mid_sync_fails")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-terminated@example.test",
    display_name = "Sync JMAP Terminated",
    account_name = "Sync JMAP Terminated",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")
local account_id = account.account_id

local armed, arm_err = client:request("test.bifrost_arm_hook", {
    account_id = account_id,
    hook = {
        kind = "force_terminated",
        recovery = "auth_lost",
    },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "force_terminated hook was not armed")

local failed, failed_err = client:start_sync({
    account_id = account_id,
}, 30)
harness.assert(failed_err == nil, "terminated start_sync transport failed")
harness.assert(failed ~= nil, "terminated start_sync returned nil result")
harness.assert_eq(failed.result, "failed", "terminated sync result")
harness.assert(failed.error ~= nil, "terminated sync missing error")

local parked, parked_err = client:start_sync({
    account_id = account_id,
}, 10)
harness.assert(parked_err == nil, "parked follow-up start_sync transport failed")
harness.assert(parked ~= nil, "parked follow-up start_sync returned nil result")
harness.assert_eq(parked.result, "failed", "parked follow-up sync result")
harness.assert(parked.error ~= nil, "parked follow-up sync missing error")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
