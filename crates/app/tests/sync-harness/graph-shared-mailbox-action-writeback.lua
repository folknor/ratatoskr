-- description: Graph shared-mailbox archive dispatches remotely and survives resync
-- expected: pass
-- fixture: shared-graph-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_shared_mailbox_action_writeback

local function wait_completed(queue, plan_id)
    local deadline = harness.now_ms() + 15000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" and event.plan_id == plan_id then
            return event
        end
    end
    return nil
end

local function state(client, account_id)
    local result, err = client:request("TestQueryDbState", { account_id = account_id, message_limit = 20 })
    harness.assert(err == nil, "TestQueryDbState failed")
    return result
end

local dir = harness.data_dir("graph_shared_mailbox_action_writeback")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local queue = client:notifications()
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
    provider = "graph", delegate_discovery_enabled = true,
})
harness.assert(account_err == nil, "seed failed")
local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil and initial.result == "completed", initial.error or "initial sync failed")
local shared = nil
for _, message in ipairs(state(client, account.account_id).messages) do
    if message.subject == "Shared Graph copy" then shared = message end
end
harness.assert(shared ~= nil, "shared message missing")

local ack, ack_err = client:request("ActionExecutePlan", { operations = {{
    account_id = account.account_id, thread_id = shared.thread_id, operation = "Archive",
}}})
harness.assert(ack_err == nil and ack.journaled, "archive plan was not journaled")
local completed = wait_completed(queue, ack.plan_id)
harness.assert(completed ~= nil, "missing action.completed")
harness.assert(completed.summary_remote_succeeded >= 1, "archive did not reach Graph")
harness.assert_eq(completed.summary_remote_failed, 0, "archive remote failure")
harness.assert_eq(completed.summary_local_only, 0, "archive degraded to local-only")
harness.assert_eq(completed.summary_conflicts, 0, "archive conflict")

local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil and resync.result == "completed", resync.error or "resync failed")
local read, read_err = client:request("TestThreadRead", { account_id = account.account_id, thread_id = shared.thread_id })
harness.assert(read_err == nil and read.exists, "shared thread missing after server round-trip")
local in_inbox = false
for _, id in ipairs(read.label_ids) do if id == "shared:alice:graph:alice-inbox" then in_inbox = true end end
harness.assert(not in_inbox, "shared thread remained in its inbox after server round-trip")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
