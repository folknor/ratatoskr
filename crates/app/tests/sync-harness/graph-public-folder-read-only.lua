-- description: Graph public-folder move and label actions fail before local mutation
-- expected: pass
-- fixture: public-folder-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_public_folder_read_only

local function wait_completed(queue, plan_id)
    local deadline = harness.now_ms() + 15000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" and event.plan_id == plan_id then return event end
    end
    return nil
end

local function execute_failed(client, queue, account_id, thread_id, operation)
    local ack, err = client:request("ActionExecutePlan", { operations = {{
        account_id = account_id, thread_id = thread_id, operation = operation,
        dest = "INBOX", label_id = "cat:blocked",
    }}})
    harness.assert(err == nil and ack.journaled, operation .. " plan was not journaled")
    local completed = wait_completed(queue, ack.plan_id)
    harness.assert(completed ~= nil, operation .. " missing action.completed")
    harness.assert_eq(completed.summary_remote_succeeded, 0, operation .. " unexpectedly reached provider")
    harness.assert_eq(completed.summary_local_only, 0, operation .. " mutated locally")
    return completed
end

local dir = harness.data_dir("graph_public_folder_read_only")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local queue = client:notifications()
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
    provider = "graph", public_folders_enabled = true, public_folder_pins = { "public:pf-notices" },
})
harness.assert(account_err == nil, "seed failed")
local synced, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and synced.result == "completed", synced.error or "sync failed")
local before, before_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(before_err == nil, "before state failed")
local notice = nil
for _, message in ipairs(before.messages) do if message.subject == "Public notice" then notice = message end end
harness.assert(notice ~= nil, "public item missing")
execute_failed(client, queue, account.account_id, notice.thread_id, "MoveToFolder")
execute_failed(client, queue, account.account_id, notice.thread_id, "AddLabel")
local after, after_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(after_err == nil, "after state failed")
harness.assert_eq(after.message_count, before.message_count, "read-only action changed local messages")
local read, read_err = client:request("TestThreadRead", { account_id = account.account_id, thread_id = notice.thread_id })
harness.assert(read_err == nil and read.exists, "read-only action removed public thread")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
