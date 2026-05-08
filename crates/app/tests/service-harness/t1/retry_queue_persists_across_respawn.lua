-- description: pending retry queue persists across Service respawn
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

local function read_pending(client, account_id, thread_id)
    local pending, pending_err = client:request("TestPendingOpsRead", {
        account_id = account_id,
        resource_id = thread_id,
        operation_type = "markRead",
    })
    harness.assert(pending_err == nil, "pending ops read failed")
    return pending
end

local function wait_for_pending_row(client, account_id, thread_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local pending = read_pending(client, account_id, thread_id)
        if pending.total == 1 and pending.pending == 1 then
            return pending
        end
        harness.sleep(100)
    end
    return nil
end

local dir = harness.data_dir("t1_retry_queue_persists")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")
local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-retry-queue@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-retry-queue-thread",
    is_read = false,
    is_chat_thread = true,
    chat_email = "m4-retry-queue-chat@example.test",
})
harness.assert(thread_err == nil, "seed thread failed")

local ack, ack_err = client:request("ActionExecutePlan", {
    operations = {
        {
            account_id = account.account_id,
            thread_id = thread.thread_id,
            operation = "SetRead",
            to = true,
        },
    },
})
harness.assert(ack_err == nil, "action.execute_plan failed")
harness.assert(ack.journaled, "plan was not journaled")

local completed = wait_for_action_completed(queue, ack.plan_id, 10)
harness.assert(completed ~= nil, "missing action.completed")

local before =
    wait_for_pending_row(client, account.account_id, thread.thread_id, 10)
harness.assert(before ~= nil, "pending op was not queued")
harness.assert_eq(before.failed, 0, "pending failed count before respawn")
local before_op = before.operations[1]
harness.assert_eq(
    before_op.operation_type,
    "markRead",
    "operation type before respawn"
)

local pid = client:child_pid()
harness.assert(pid ~= nil, "pid missing")
harness.kill(pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 30)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(
    harness.same_client(client, respawn_first.client),
    "client Arc changed"
)

local respawn_second = next_lifecycle(events, 30)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")

local after =
    wait_for_pending_row(client, account.account_id, thread.thread_id, 10)
harness.assert(after ~= nil, "pending op missing after respawn")
harness.assert_eq(after.failed, 0, "pending failed count after respawn")
local after_op = after.operations[1]
harness.assert_eq(after_op.id, before_op.id, "pending op id")
harness.assert_eq(
    after_op.account_id,
    before_op.account_id,
    "pending op account"
)
harness.assert_eq(
    after_op.resource_id,
    before_op.resource_id,
    "pending op resource"
)
harness.assert_eq(
    after_op.operation_type,
    before_op.operation_type,
    "pending op type"
)
harness.assert(
    after_op.retry_count >= before_op.retry_count,
    "retry count moved backwards"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
