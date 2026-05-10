-- description: IMAP delta imports scripted fixture new/change steps
-- expected: pass
-- fixture: jmap-incremental.lua
-- protocol: imap
-- ceiling: 120s

local function message_by_subject(state, subject)
    for _, message in ipairs(state.messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function has_value(values, expected)
    for _, value in ipairs(values) do
        if value == expected then
            return true
        end
    end
    return false
end

local function fixture_email_by_id(snapshot, id)
    for _, email in ipairs(snapshot.emails) do
        if email.id == id then
            return email
        end
    end
    return nil
end

local function query_state(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
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
    return response
end

local function fixture_snapshot(endpoint)
    local snapshot = harness.snapshot_state(endpoint)
    harness.assert_eq(snapshot.name, "jmap-incremental", "fixture snapshot name")
    return snapshot
end

local function assert_imap_delta_checked(endpoint, label)
    local requests = harness.mock_requests(endpoint, { stable = true })
    harness.assert(
        harness.request_count(requests, "imap", "UID SEARCH") >= 1,
        label .. " did not search IMAP UIDs"
    )
    return requests
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_incremental_new_change")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-incremental@example.test",
    display_name = "Sync IMAP Incremental",
    account_name = "Sync IMAP Incremental",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_sync(client, account.account_id, "initial")
local initial_snapshot = fixture_snapshot(admin_endpoint)
harness.assert(
    fixture_email_by_id(initial_snapshot, "email-001") ~= nil,
    "remote snapshot missing email-001"
)
harness.assert(
    fixture_email_by_id(initial_snapshot, "email-002") ~= nil,
    "remote snapshot missing email-002"
)

local initial = query_state(client, account.account_id, "initial")
harness.assert_eq(initial.message_count, 2, "initial message count")
harness.assert(message_by_subject(initial, "Welcome") ~= nil, "missing Welcome")
harness.assert(message_by_subject(initial, "Status update") ~= nil, "missing Status update")

harness.clear_mock_requests(admin_endpoint)
local new_step = apply_step(admin_endpoint, "new")
harness.assert_eq(new_step.changes.emails.created[1], "email-003", "new step created id")
local remote_after_new = fixture_snapshot(admin_endpoint)
local remote_lunch = fixture_email_by_id(remote_after_new, "email-003")
harness.assert(remote_lunch ~= nil, "remote snapshot missing email-003")
run_sync(client, account.account_id, "new step")
local new_requests = assert_imap_delta_checked(admin_endpoint, "new step")
harness.assert(
    harness.request_count(new_requests, "imap", "UID FETCH") >= 1,
    "new step did not fetch the new IMAP message"
)
local after_new = query_state(client, account.account_id, "after new")
harness.assert_eq(after_new.message_count, 3, "message count after new")
harness.assert(message_by_subject(after_new, "Lunch?") ~= nil, "new message missing")

harness.clear_mock_requests(admin_endpoint)
local change_step = apply_step(admin_endpoint, "change")
harness.assert_eq(change_step.changes.emails.updated[1], "email-002", "change step updated id")
local remote_after_change = fixture_snapshot(admin_endpoint)
local remote_status = fixture_email_by_id(remote_after_change, "email-002")
harness.assert(remote_status ~= nil, "remote snapshot missing email-002 after change")
harness.assert(has_value(remote_status.keywords, "$seen"), "remote email-002 did not gain $seen")
harness.assert(
    has_value(remote_status.keywords, "$flagged"),
    "remote email-002 did not gain $flagged"
)
run_sync(client, account.account_id, "change step")
assert_imap_delta_checked(admin_endpoint, "change step")
local after_change = query_state(client, account.account_id, "after change")
local status_update = message_by_subject(after_change, "Status update")
harness.assert(status_update ~= nil, "Status update missing after change")
harness.assert(status_update.is_read, "Status update did not import $seen")
harness.assert(status_update.is_starred, "Status update did not import $flagged")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
