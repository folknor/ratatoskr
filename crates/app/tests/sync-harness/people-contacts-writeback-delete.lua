-- description: Google People API contact writeback and delete hit provider routes
-- expected: pass
-- fixture: graph-contacts-small.toml
-- protocol: people
-- ceiling: 120s

local CONTACT_DELTA = "GET /v1/people/me/connections"

local function contact_by_email(contacts, email)
    for _, contact in ipairs(contacts) do
        if contact.email == email then
            return contact
        end
    end
    return nil
end

local function query_state(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        contact_limit = 10,
    })
    harness.assert(err == nil, label .. " TestQueryDbState failed")
    return state
end

local function run_sync(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local function run_until_contact_delta(client, endpoint, account_id, label)
    -- 20 is the production Gmail contact-delta cadence.
    for i = 1, 20 do
        run_sync(client, account_id, label .. " delta cycle " .. i)
        local requests = harness.mock_requests(endpoint, { stable = true })
        if harness.request_count(requests, "people", CONTACT_DELTA) >= 1 then
            return requests, i
        end
    end
    harness.assert(false, label .. " did not call People contact delta within 20 syncs")
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_people_contacts_writeback_delete")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-people-contacts-writeback@example.test",
    display_name = "Sync People Contacts Writeback",
    account_name = "Sync People Contacts Writeback",
    provider = "gmail_api",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
run_sync(client, account.account_id, "initial")
harness.marker("SYNC_END")
local initial = query_state(client, account.account_id, "initial")
harness.assert_eq(initial.contact_count, 3, "initial contact count")
local bob = contact_by_email(initial.contacts, "bob@example.com")
harness.assert(bob ~= nil, "missing Bob contact")
harness.assert_eq(bob.source, "google", "Bob source")
harness.assert_eq(bob.server_id, "people/contact-002", "Bob server id")

harness.clear_mock_requests(admin_endpoint)
local saved, save_err = client:request("contacts.contact_save_with_writeback", {
    id = bob.id,
    email = bob.email,
    display_name = bob.display_name,
    phone = "+1 555 0102",
    company = "Bell Labs",
    notes = "updated through People writeback",
    account_id = account.account_id,
    source = bob.source,
    server_id = bob.server_id,
    groups = {},
})
harness.assert(save_err == nil, "contacts.contact_save_with_writeback failed")
harness.assert_eq(saved.writeback.kind, "success", "save writeback result")

local after_save = query_state(client, account.account_id, "after save")
local saved_bob = contact_by_email(after_save.contacts, "bob@example.com")
harness.assert(saved_bob ~= nil, "Bob missing after save")
harness.assert_eq(saved_bob.phone, "+1 555 0102", "Bob phone")
harness.assert_eq(saved_bob.company, "Bell Labs", "Bob company")
harness.assert_eq(saved_bob.notes, "updated through People writeback", "Bob notes")
harness.assert_eq(saved_bob.server_id, "people/contact-002", "Bob server id after save")

local update_requests = harness.mock_requests(admin_endpoint, { stable = true })
local patch_requests = harness.request_count(
    update_requests,
    "people",
    "PATCH /v1/people/contact-002:updateContact"
)
harness.assert(patch_requests >= 1, "People writeback did not PATCH contact")

harness.clear_mock_requests(admin_endpoint)
local deleted, delete_err = client:request("contacts.contact_delete", {
    id = saved_bob.id,
})
harness.assert(delete_err == nil, "contacts.contact_delete failed")
harness.assert_eq(deleted.writeback.kind, "success", "delete writeback result")

local after_delete = query_state(client, account.account_id, "after delete")
harness.assert_eq(after_delete.contact_count, 2, "contact count after delete")
harness.assert(
    contact_by_email(after_delete.contacts, "bob@example.com") == nil,
    "Bob survived local delete"
)

local delete_requests = harness.mock_requests(admin_endpoint, { stable = true })
local provider_delete_requests = harness.request_count(
    delete_requests,
    "people",
    "DELETE /v1/people/contact-002:deleteContact"
)
harness.assert(provider_delete_requests >= 1, "People delete did not DELETE contact")

harness.clear_mock_requests(admin_endpoint)
harness.marker("SYNC_START")
local delta_requests, measured_syncs =
    run_until_contact_delta(client, admin_endpoint, account.account_id, "post-delete")
harness.marker("SYNC_END")
local after_delta = query_state(client, account.account_id, "after delta")
harness.assert_eq(after_delta.contact_count, 2, "contact count after post-delete delta")
harness.assert(
    contact_by_email(after_delta.contacts, "bob@example.com") == nil,
    "Bob returned after post-delete delta"
)

harness.write_summary({
    correct = 1,
    measured_syncs = measured_syncs + 1,
    contact_count = after_delta.contact_count,
    provider_requests = #update_requests + #delete_requests + #delta_requests,
    people_patch_requests = patch_requests,
    people_delete_requests = provider_delete_requests,
    people_contact_delta_requests = harness.request_count(
        delta_requests,
        "people",
        CONTACT_DELTA
    ),
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
