-- description: B8 Graph contact update/delete use engine passthroughs
-- expected: pass
-- fixture: graph-contacts-small.toml
-- protocol: graph
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")

local client, spawn_err = harness.spawn(harness.data_dir("b8_writeback_graph"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", { email = "b8-w-graph@example.test", provider = "graph" })
harness.assert(seed_err == nil, "seed Graph account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "contact pull")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 10 })
harness.assert(state_err == nil, "snapshot")
local contact = state.contacts[1]
harness.assert(contact ~= nil, "contact")

local saved, save_err = client:request("contacts.contact_save_with_writeback", {
    id = contact.id, email = contact.email, display_name = contact.display_name,
    phone = "+1 555 0103", company = "Bifrost", notes = "Graph writeback",
    account_id = account.account_id, source = contact.source, server_id = contact.server_id, groups = {},
})
harness.assert(save_err == nil, "save")
harness.assert_eq(saved.writeback.kind, "success", "save outcome")

local deleted, delete_err = client:request("contacts.contact_delete", { id = contact.id })
harness.assert(delete_err == nil, "delete")
harness.assert_eq(deleted.writeback.kind, "success", "delete outcome")

harness.write_summary({ correct = 1, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
