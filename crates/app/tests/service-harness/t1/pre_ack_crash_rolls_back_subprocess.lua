-- description: pre-ack crash before journal write rolls back optimistic plan
-- ceiling: 120s

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local dir = harness.data_dir("t1_pre_ack_crash_rolls_back_subprocess")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-pre-ack@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-pre-ack-thread",
})
harness.assert(thread_err == nil, "seed thread failed")

local _, delay_err = client:request("TestDelayNextWrite", {
    kind = "action.before_journal_write",
    millis = 2000,
})
harness.assert(delay_err == nil, "delay hook failed")

local plan_id = "00000000-0000-4000-8000-000000000401"
local pending = client:request_async("ActionExecutePlan", {
    plan_id = plan_id,
    operations = {
        {
            account_id = account.account_id,
            thread_id = thread.thread_id,
            operation = "SetPinned",
            to = true,
        },
    },
})

harness.sleep(200)
local pid = client:child_pid()
harness.assert(pid ~= nil, "pid missing")
harness.kill(pid, "SIGKILL")

local result, request_err = pending:await(5)
harness.assert(result == nil, "pre-ack request unexpectedly succeeded")
harness.assert(request_err ~= nil, "pre-ack request missing error")
harness.assert_eq(request_err.kind, "ServiceCrashed", "pre-ack request error")

local respawn_first = next_lifecycle(events, 30)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "client Arc changed")

local respawn_second = next_lifecycle(events, 30)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")

local status, status_err = client:request("ActionJobStatus", {
    plan_id = plan_id,
})
harness.assert(status_err == nil, "job status failed")
harness.assert_eq(status.kind, "not_found", "pre-ack job status")

local read, read_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread.thread_id,
})
harness.assert(read_err == nil, "thread read failed")
harness.assert(not read.is_pinned, "pre-ack plan was applied")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
