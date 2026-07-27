-- description: Pinned Graph public mail folders persist real namespaced threads
-- expected: pass
-- fixture: public-folder-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_public_folder

local dir = harness.data_dir("graph_public_folder")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
    provider = "graph", public_folders_enabled = true,
    public_folder_pins = { "public:pf-notices", "public:pf-calendar" },
})
harness.assert(account_err == nil, "seed failed")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and completed.result == "completed", completed.error or "sync failed")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(state_err == nil, "state read failed")
local notice, appointment = nil, nil
for _, message in ipairs(state.messages) do
    if message.subject == "Public notice" then notice = message end
    if message.subject == "Public appointment" then appointment = message end
end
harness.assert(notice ~= nil and notice.id ~= "notice-1", "public mail item missing or unqualified")
harness.assert(appointment == nil, "non-mail public item entered mail path")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
