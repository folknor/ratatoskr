-- description: mark_chat_read quiet job emits only action.completed
-- ceiling: 90s

local function count_action_events(events, plan_id)
    local completed = 0
    local outcomes = 0
    for _, event in ipairs(events) do
        if event.plan_id == plan_id and event.type == "ActionCompleted" then
            completed = completed + 1
        end
        if event.plan_id == plan_id and event.type == "OperationOutcome" then
            outcomes = outcomes + 1
        end
    end
    return completed, outcomes
end

local dir = harness.data_dir("t1_mark_chat_read_quiet")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-chat-owner@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m4-chat-thread",
    is_read = false,
    is_chat_thread = true,
    chat_email = "m4-chat@example.test",
})
harness.assert(thread_err == nil, "seed thread failed")

local ack, ack_err = client:request("ActionMarkChatRead", {
    chat_email = "m4-chat@example.test",
})
harness.assert(ack_err == nil, "mark_chat_read failed")
harness.assert(ack.journaled, "mark_chat_read was not journaled")

local events = queue:drain_for(5)
local completed, outcomes = count_action_events(events, ack.job_id)
harness.assert_eq(completed, 1, "action.completed count")
harness.assert_eq(outcomes, 0, "operation_outcome count")

local read, read_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread.thread_id,
})
harness.assert(read_err == nil, "thread read failed")
harness.assert(read.is_read, "chat thread was not marked read")
harness.assert_eq(read.unread_messages, 0, "unread message count")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
