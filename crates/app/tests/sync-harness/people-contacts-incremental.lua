-- description: Google People API contact deltas import scripted new/change/delete steps
-- expected: pass
-- fixture: graph-contacts-incremental.lua
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

local function apply_step(endpoint, step_id)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "test/fixture/step"),
        body = {
            expect = step_id,
        },
    })
    harness.assert(response.ok, "fixture step failed")
    harness.assert_eq(response.step, step_id, "fixture step id")
    harness.assert_eq(response.applied, 1, "fixture step applied count")
end

local measured_sync_count = 0
local summary_provider_requests = 0
local summary_contact_delta_requests = 0

local function run_measured_until_contact_delta(client, endpoint, account_id, label)
    harness.marker("SYNC_START")
    local requests, sync_count = run_until_contact_delta(client, endpoint, account_id, label)
    harness.marker("SYNC_END")
    measured_sync_count = measured_sync_count + sync_count
    summary_provider_requests = summary_provider_requests + #requests
    summary_contact_delta_requests =
        summary_contact_delta_requests + harness.request_count(requests, "people", CONTACT_DELTA)
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_people_contacts_incremental")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-people-contacts-delta@example.test",
    display_name = "Sync People Contacts Delta",
    account_name = "Sync People Contacts Delta",
    provider = "gmail_api",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_sync(client, account.account_id, "initial")
local initial = query_state(client, account.account_id, "initial")
harness.assert_eq(initial.contact_count, 2, "initial contact count")
harness.assert(contact_by_email(initial.contacts, "alice@example.com") ~= nil, "missing Alice")
harness.assert(contact_by_email(initial.contacts, "bob@example.com") ~= nil, "missing Bob")

apply_step(admin_endpoint, "new")
harness.clear_mock_requests(admin_endpoint)
run_measured_until_contact_delta(client, admin_endpoint, account.account_id, "new step")
local after_new = query_state(client, account.account_id, "after new")
harness.assert_eq(after_new.contact_count, 3, "contact count after new")
local carol = contact_by_email(after_new.contacts, "carol@example.com")
harness.assert(carol ~= nil, "new Carol contact missing")
harness.assert_eq(carol.display_name, "Carol Carver", "Carol display name")
harness.assert_eq(carol.server_id, "people/contact-003", "Carol server id")

apply_step(admin_endpoint, "change")
harness.clear_mock_requests(admin_endpoint)
run_measured_until_contact_delta(client, admin_endpoint, account.account_id, "change step")
local after_change = query_state(client, account.account_id, "after change")
harness.assert_eq(after_change.contact_count, 3, "contact count after change")
local bob = contact_by_email(after_change.contacts, "bob@example.com")
harness.assert(bob ~= nil, "Bob contact missing after change")
harness.assert_eq(bob.display_name, "Robert Bell", "Bob display name after change")
harness.assert(
    contact_by_email(after_change.contacts, "robert@work.example") == nil,
    "People sync should keep only Bob's primary email"
)

apply_step(admin_endpoint, "delete")
harness.clear_mock_requests(admin_endpoint)
run_measured_until_contact_delta(client, admin_endpoint, account.account_id, "delete step")
local after_delete = query_state(client, account.account_id, "after delete")
harness.assert_eq(after_delete.contact_count, 2, "contact count after delete")
harness.assert(
    contact_by_email(after_delete.contacts, "alice@example.com") == nil,
    "Alice survived delete"
)
harness.assert(contact_by_email(after_delete.contacts, "bob@example.com") ~= nil, "Bob lost")
harness.assert(contact_by_email(after_delete.contacts, "carol@example.com") ~= nil, "Carol lost")

local end_response = harness.http_json({
    method = "POST",
    url = harness.join_url(admin_endpoint, "test/fixture/step"),
    body = {},
})
harness.assert(end_response.ok, "end-of-script response failed")
harness.assert(end_response.step == nil, "end-of-script step should be nil")
harness.assert(not end_response.applied, "end-of-script should not apply")

harness.write_summary({
    correct = 1,
    measured_syncs = measured_sync_count,
    contact_count = after_delete.contact_count,
    provider_requests = summary_provider_requests,
    people_contact_delta_requests = summary_contact_delta_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
