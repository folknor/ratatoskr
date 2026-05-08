-- description: archive 200-thread action plan under the harness budget
-- ceiling: 120s

local TOTAL = 200
local BUDGET_MS = 45000

local function has_label(thread, label_id)
    for _, value in ipairs(thread.label_ids) do
        if value == label_id then
            return true
        end
    end
    return false
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

local dir = harness.data_dir("t1_bulk_archive_200_threads")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "m4-bulk-archive@example.test",
    provider = "harness-offline",
})
harness.assert(account_err == nil, "seed account failed")

local thread_ids = {}
local operations = {}
for i = 1, TOTAL do
    local thread_id = string.format("m4-bulk-archive-%03d", i)
    local thread, thread_err = client:request("TestSeedThread", {
        account_id = account.account_id,
        thread_id = thread_id,
        label_ids = { "INBOX" },
    })
    harness.assert(thread_err == nil, "seed thread failed")
    thread_ids[i] = thread.thread_id
    operations[i] = {
        operation_id = i - 1,
        account_id = account.account_id,
        thread_id = thread.thread_id,
        operation = "Archive",
    }
end

local started = harness.now_ms()
local ack, ack_err = client:request("ActionExecutePlan", {
    operations = operations,
})
harness.assert(ack_err == nil, "action.execute_plan failed")
harness.assert(ack.journaled, "plan was not journaled")

local completed = wait_for_action_completed(queue, ack.plan_id, 45)
local elapsed = harness.now_ms() - started
harness.assert(completed ~= nil, "missing action.completed")
harness.assert_eq(completed.summary_total, TOTAL, "summary total")
harness.assert(
    elapsed < BUDGET_MS,
    "bulk archive exceeded budget; elapsed_ms=" .. elapsed
)

for i = 1, TOTAL do
    local read, read_err = client:request("TestThreadRead", {
        account_id = account.account_id,
        thread_id = thread_ids[i],
    })
    harness.assert(read_err == nil, "thread read failed")
    harness.assert(read.exists, "archived thread missing")
    harness.assert(
        not has_label(read, "INBOX"),
        "archive left INBOX on " .. thread_ids[i]
    )
end

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
