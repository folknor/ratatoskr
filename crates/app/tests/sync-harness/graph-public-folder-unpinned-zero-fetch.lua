-- description: Unpinned Graph public folders discover hierarchy without syncing items
-- expected: pass
-- fixture: public-folder-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_public_folder_unpinned

local dir = harness.data_dir("graph_public_unpinned")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
    provider = "graph", public_folders_enabled = true,
})
harness.assert(account_err == nil, "seed failed")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and completed.result == "completed", completed.error or "sync failed")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(state_err == nil, "state read failed")
harness.assert(#state.messages == 0, "unpinned public folders must not fetch items")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
