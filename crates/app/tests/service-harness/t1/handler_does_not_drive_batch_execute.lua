-- description: action.execute_plan acks without driving batch_execute inline
-- ceiling: 90s

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

local dir = harness.data_dir("t1_handler_does_not_drive_batch_execute")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-handler@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-handler-thread",
})
harness.assert(thread_err == nil, "seed thread failed")

local _, delay_err = client:request("TestDelayNextWrite", {
    kind = "action.batch_execute",
    millis = 1500,
})
harness.assert(delay_err == nil, "delay hook failed")

local started = harness.now_ms()
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
local elapsed = harness.now_ms() - started
harness.assert(ack_err == nil, "action.execute_plan failed")
harness.assert(ack.journaled, "plan was not journaled")
harness.assert(elapsed < 1000, "handler drove delayed batch_execute inline")

local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "health.ping failed during delayed worker")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")

local completed = wait_for_action_completed(queue, ack.plan_id, 10)
harness.assert(completed ~= nil, "missing action.completed")

local read, read_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread.thread_id,
})
harness.assert(read_err == nil, "thread read failed")
harness.assert(read.is_pinned, "worker did not pin thread")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
