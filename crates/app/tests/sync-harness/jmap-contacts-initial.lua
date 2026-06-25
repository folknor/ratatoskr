-- description: JMAP contact initial sync imports ContactCard fixtures
-- expected: pass
-- fixture: graph-contacts-small.toml
-- protocol: jmap
-- ceiling: 120s

local function contact_by_email(contacts, email)
    for _, contact in ipairs(contacts) do
        if contact.email == email then
            return contact
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_contacts_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-contacts@example.test",
    display_name = "Sync JMAP Contacts",
    account_name = "Sync JMAP Contacts",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    contact_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert(state.contact_count >= 1, "contact count")

local alice = contact_by_email(state.contacts, "alice@example.com")
harness.assert(alice ~= nil, "missing Alice contact")
harness.assert_eq(alice.source, "jmap", "Alice source")
harness.assert_eq(alice.account_id, account.account_id, "Alice account")

local requests = harness.mock_requests(admin_endpoint)
local contact_get_requests = harness.request_count(requests, "jmap", "ContactCard/get")
harness.assert(contact_get_requests >= 1, "JMAP contact sync did not call ContactCard/get")

harness.write_summary({
    correct = 1,
    contact_count = state.contact_count,
    provider_requests = #requests,
    jmap_contact_get_requests = contact_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
