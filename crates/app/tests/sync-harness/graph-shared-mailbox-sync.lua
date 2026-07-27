-- description: Graph delegate mailbox sync persists foreign mail
-- expected: pass
-- fixture: shared-graph-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_shared_mailbox

local dir = harness.data_dir("graph_shared_mailbox")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
    provider = "graph", delegate_discovery_enabled = true,
})
harness.assert(account_err == nil, "seed failed")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and completed.result == "completed", completed.error or "sync failed")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(state_err == nil, "state read failed")
local shared = nil
for _, message in ipairs(state.messages) do
    if message.subject == "Shared Graph copy" then shared = message end
end
harness.assert(shared ~= nil and shared.id ~= "alice-message", "foreign Graph message missing or unqualified")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
