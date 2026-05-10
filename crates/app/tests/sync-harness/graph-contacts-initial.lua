-- description: Graph contact initial sync imports contact-folder fixtures
-- expected: pass
-- fixture: graph-contacts-small.toml
-- protocol: graph
-- ceiling: 120s

local function contact_by_email(contacts, email)
    for _, contact in ipairs(contacts) do
        if contact.email == email then
            return contact
        end
    end
    return nil
end

local function contact_by_server_id(contacts, server_id)
    for _, contact in ipairs(contacts) do
        if contact.server_id == server_id then
            return contact
        end
    end
    return nil
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_graph_contacts_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-graph-contacts@example.test",
    display_name = "Sync Graph Contacts",
    account_name = "Sync Graph Contacts",
    provider = "graph",
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
    contact_group_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.contact_count, 4, "contact count")
harness.assert_eq(state.contact_group_count, 0, "contact group count")

local alice = contact_by_email(state.contacts, "alice@example.com")
harness.assert(alice ~= nil, "missing Alice primary email")
harness.assert_eq(alice.display_name, "Alice Anderson", "Alice display name")
harness.assert_eq(alice.source, "graph", "Alice source")
harness.assert_eq(alice.account_id, account.account_id, "Alice account")
harness.assert_eq(alice.server_id, "contact-001", "Alice server id")

local alice_alt = contact_by_email(state.contacts, "alice.anderson@example.org")
harness.assert(alice_alt ~= nil, "missing Alice alternate email")
harness.assert_eq(alice_alt.display_name, "Alice Anderson", "Alice alternate name")
harness.assert_eq(alice_alt.server_id, "contact-001", "Alice alternate server id")

local bob = contact_by_email(state.contacts, "bob@example.com")
harness.assert(bob ~= nil, "missing Bob contact")
harness.assert_eq(bob.display_name, "Bob Bell", "Bob display name")
harness.assert_eq(bob.source, "graph", "Bob source")
harness.assert_eq(bob.server_id, "contact-002", "Bob server id")

local vendor = contact_by_email(state.contacts, "sales@acme.example")
harness.assert(vendor ~= nil, "missing vendor contact")
harness.assert_eq(vendor.display_name, "Acme Supplies", "vendor display name")
harness.assert_eq(vendor.server_id, "contact-100", "vendor server id")

harness.assert(
    contact_by_server_id(state.contacts, "contact-003") == nil,
    "contact with no email should be skipped"
)

local requests = harness.mock_requests(admin_endpoint)
local folder_list_requests =
    harness.request_count(requests, "graph", "GET /v1.0/me/contactFolders")
local default_contact_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/contactFolders/cf-default/contacts"
)
local vendor_contact_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/contactFolders/cf-vendors/contacts"
)
local default_delta_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/contactFolders/cf-default/contacts/delta"
)
local vendor_delta_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/contactFolders/cf-vendors/contacts/delta"
)
harness.assert(
    folder_list_requests >= 1,
    "Graph contact sync did not list contact folders"
)
harness.assert(
    default_contact_requests >= 1,
    "Graph contact sync did not list default-folder contacts"
)
harness.assert(
    vendor_contact_requests >= 1,
    "Graph contact sync did not list vendor-folder contacts"
)
harness.assert(
    default_delta_requests >= 1,
    "Graph contact sync did not bootstrap default-folder delta"
)
harness.assert(
    vendor_delta_requests >= 1,
    "Graph contact sync did not bootstrap vendor-folder delta"
)

harness.write_summary({
    correct = 1,
    contact_count = state.contact_count,
    contact_group_count = state.contact_group_count,
    provider_requests = #requests,
    graph_contact_folder_list_requests = folder_list_requests,
    graph_default_contact_requests = default_contact_requests,
    graph_vendor_contact_requests = vendor_contact_requests,
    graph_default_delta_requests = default_delta_requests,
    graph_vendor_delta_requests = vendor_delta_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
