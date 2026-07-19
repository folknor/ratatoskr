-- description: B8 CardDAV update/delete are no longer LocalOnly stubs
-- expected: pass
-- fixture: carddav-contacts-small.toml
-- protocol: carddav
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
local dav = harness.env("RATATOSKR_TEST_CARDDAV_ENDPOINT") or harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(dav ~= nil, "CardDAV endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("b8_writeback_carddav"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-w-carddav@example.test",
    provider = "imap",
    caldav_url = dav,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(seed_err == nil, "seed IMAP/CardDAV account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "contact pull")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 10 })
harness.assert(state_err == nil, "snapshot")
local contact = state.contacts[1]
harness.assert(contact ~= nil, "attached CardDAV contact")

local saved, save_err = client:request("contacts.contact_save_with_writeback", {
    id = contact.id, email = contact.email, display_name = contact.display_name,
    phone = "+1 555 0104", company = "Bifrost", notes = "CardDAV writeback",
    account_id = account.account_id, source = contact.source, server_id = contact.server_id, groups = {},
})
harness.assert(save_err == nil, "save")
harness.assert_eq(saved.writeback.kind, "success", "CardDAV save must succeed")

local deleted, delete_err = client:request("contacts.contact_delete", { id = contact.id })
harness.assert(delete_err == nil, "delete")
harness.assert_eq(deleted.writeback.kind, "success", "CardDAV delete must succeed")

harness.write_summary({ correct = 1, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
