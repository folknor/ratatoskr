-- description: journaled action replays after SIGKILL and respawn
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

local function wait_for_action_completed(queue, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" then
            if event.plan_id == plan_id then
                return event
            end
        end
    end
    return nil
end

local dir = harness.data_dir("t1_journal_replays_after_respawn")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")
local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-replay@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-replay-thread",
})
harness.assert(thread_err == nil, "seed thread failed")

local _, delay_err = client:request("TestDelayNextWrite", {
    kind = "action.batch_execute",
    millis = 2000,
})
harness.assert(delay_err == nil, "delay hook failed")

local ack, ack_err = client:request("ActionExecutePlan", {
    operations = {
        {
            account_id = account.account_id,
            thread_id = thread.thread_id,
            operation = "SetPinned",
            to = true,
        },
    },
})
harness.assert(ack_err == nil, "action.execute_plan failed")

local pid = client:child_pid()
harness.assert(pid ~= nil, "pid missing")
harness.kill(pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 30)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "client Arc changed")

local respawn_second = next_lifecycle(events, 30)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")

local completed = wait_for_action_completed(queue, ack.plan_id, 20)
harness.assert(completed ~= nil, "missing replayed action.completed")

local status, status_err = client:request("ActionJobStatus", {
    plan_id = ack.plan_id,
})
harness.assert(status_err == nil, "job status failed")
harness.assert_eq(status.kind, "journaled", "job status kind")
harness.assert_eq(status.status, "completed", "job status")

local read, read_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread.thread_id,
})
harness.assert(read_err == nil, "thread read failed")
harness.assert(read.is_pinned, "replayed worker did not pin thread")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
