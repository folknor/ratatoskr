-- description: M3 test helpers seed account, counters, and crash rule
-- ceiling: 60s

local dir = harness.data_dir("m3_test_helpers_smoke")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m3-harness@example.test",
    display_name = "M3 Harness",
    account_name = "M3 Harness",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")
harness.assert(account.account_id ~= nil, "seeded account id missing")
harness.assert_eq(account.email, "m3-harness@example.test", "seeded email")
harness.assert(account.label_count > 0, "seeded account has no labels")

local counter, counter_err = client:request("TestCounterRead", {
    counter = "action.batch_execute",
})
harness.assert(counter_err == nil, "TestCounterRead failed")
harness.assert_eq(counter.counter, "action.batch_execute", "counter name")
harness.assert(counter.value >= 0, "counter value missing")

local _, crash_err = client:request("TestCrashAfterNWrites", {
    kind = "action.batch_execute",
    n = 1000000,
})
harness.assert(crash_err == nil, "TestCrashAfterNWrites failed")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
