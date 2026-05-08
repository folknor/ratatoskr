-- description: queued action outcomes from a prior generation are dropped
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

local function wait_for_completed_status(client, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local status, status_err = client:request("ActionJobStatus", {
            plan_id = plan_id,
        })
        harness.assert(status_err == nil, "job status failed")
        if status.kind == "journaled" and status.status == "completed" then
            return status
        end
        harness.sleep(100)
    end
    return nil
end

local function wait_for_action_completed(queue, client, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" then
            if event.plan_id == plan_id then
                harness.assert(
                    client:notification_should_dispatch(event),
                    "fresh action.completed was rejected"
                )
                return event
            end
        end
    end
    return nil
end

local dir = harness.data_dir("t1_stale_outcomes_dropped")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")
local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-stale@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-stale-thread",
})
harness.assert(thread_err == nil, "seed thread failed")

local initial_generation = client:current_generation()
local stale_ack, stale_ack_err = client:request("ActionExecutePlan", {
    operations = {
        {
            account_id = account.account_id,
            thread_id = thread.thread_id,
            operation = "SetPinned",
            to = true,
        },
    },
})
harness.assert(stale_ack_err == nil, "stale plan action.execute_plan failed")

local status = wait_for_completed_status(client, stale_ack.plan_id, 10)
harness.assert(status ~= nil, "stale plan did not complete before respawn")
harness.sleep(200)

local initial_pid = client:child_pid()
harness.assert(initial_pid ~= nil, "initial pid missing")
harness.kill(initial_pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 30)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "client Arc changed")

local respawn_second = next_lifecycle(events, 30)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")

local respawn_generation = client:current_generation()
harness.assert(
    respawn_generation > initial_generation,
    "generation did not advance after respawn"
)

local stale_events = queue:drain_for(1)
local stale_count = 0
for _, event in ipairs(stale_events) do
    if event.plan_id == stale_ack.plan_id then
        harness.assert(
            event.type == "OperationOutcome" or event.type == "ActionCompleted",
            "unexpected stale event type"
        )
        harness.assert_eq(
            event.service_generation,
            initial_generation,
            "stale event generation"
        )
        harness.assert(
            not client:notification_should_dispatch(event),
            "stale event would dispatch after respawn"
        )
        stale_count = stale_count + 1
    end
end
harness.assert(stale_count > 0, "no stale action notifications were queued")

local fresh_ack, fresh_ack_err = client:request("ActionExecutePlan", {
    operations = {
        {
            account_id = account.account_id,
            thread_id = thread.thread_id,
            operation = "SetPinned",
            to = false,
        },
    },
})
harness.assert(fresh_ack_err == nil, "fresh action.execute_plan failed")

local fresh_completed =
    wait_for_action_completed(queue, client, fresh_ack.plan_id, 10)
harness.assert(fresh_completed ~= nil, "missing fresh action.completed")
harness.assert_eq(
    fresh_completed.service_generation,
    respawn_generation,
    "fresh event generation"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
