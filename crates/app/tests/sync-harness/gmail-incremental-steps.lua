-- description: Gmail history steps converge deletes and label-only updates
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: jmap-incremental.toml
-- protocol: gmail
-- ceiling: 120s

local function message_by_id(state, id)
    for _, message in ipairs(state.messages) do
        if message.id == id then
            return message
        end
    end
    return nil
end

local function assert_has_value(values, expected, message)
    for _, value in ipairs(values) do
        if value == expected then
            return
        end
    end
    harness.assert(false, message)
end

local function assert_lacks_value(values, expected, message)
    for _, value in ipairs(values) do
        if value == expected then
            harness.assert(false, message)
        end
    end
end

local function fixture_email_by_id(snapshot, id)
    for _, email in ipairs(snapshot.emails) do
        if email.id == id then
            return email
        end
    end
    return nil
end

local function fixture_snapshot(endpoint)
    local snapshot = harness.snapshot_state(endpoint)
    harness.assert_eq(snapshot.name, "jmap-incremental", "fixture snapshot name")
    return snapshot
end

local function query_state(client, account_id)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, "TestQueryDbState failed")
    return state
end

local function run_delta(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local measured_sync_count = 0

local function run_measured_delta(client, account_id, label)
    harness.marker("SYNC_START")
    run_delta(client, account_id, label)
    harness.marker("SYNC_END")
    measured_sync_count = measured_sync_count + 1
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

local function assert_delta_path(endpoint, label)
    local requests = harness.mock_requests(endpoint, { stable = true })
    harness.assert(
        harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/history") >= 1,
        label .. " did not call Gmail history"
    )
    return requests
end

local summary_provider_requests = 0
local summary_history = 0
local summary_message_get = 0

local function record_gmail_requests(requests)
    summary_provider_requests = summary_provider_requests + #requests
    summary_history =
        summary_history + harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/history")
    summary_message_get =
        summary_message_get + harness.request_count_prefix(requests, "gmail", "GET /gmail/v1/users/me/messages/")
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-incremental-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local gmail_endpoint = harness.env("RATATOSKR_TEST_GMAIL_ENDPOINT")
harness.assert(gmail_endpoint ~= nil, "RATATOSKR_TEST_GMAIL_ENDPOINT missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_incremental_steps")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-incremental@example.test",
    display_name = "Sync Gmail Incremental",
    account_name = "Sync Gmail Incremental",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-incremental-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_delta(client, account.account_id, "initial")
local initial_snapshot = fixture_snapshot(admin_endpoint)
harness.assert(
    fixture_email_by_id(initial_snapshot, "email-001") ~= nil,
    "remote snapshot missing email-001"
)
harness.assert(
    fixture_email_by_id(initial_snapshot, "email-002") ~= nil,
    "remote snapshot missing email-002"
)
local initial = query_state(client, account.account_id)
harness.assert_eq(initial.message_count, 2, "initial message count")
harness.assert(message_by_id(initial, "email-001") ~= nil, "missing email-001")
harness.assert(message_by_id(initial, "email-002") ~= nil, "missing email-002")

harness.clear_mock_requests(admin_endpoint)
local new_step = apply_step(admin_endpoint, "new")
harness.assert_eq(
    new_step.changes.emails.created[1],
    "email-003",
    "new step created id"
)
run_measured_delta(client, account.account_id, "new step")
record_gmail_requests(assert_delta_path(admin_endpoint, "new step"))
local after_new = query_state(client, account.account_id)
harness.assert_eq(after_new.message_count, 3, "message count after new")
local lunch = message_by_id(after_new, "email-003")
harness.assert(lunch ~= nil, "new email missing after delta")
harness.assert_eq(lunch.subject, "Lunch?", "new email subject")

harness.clear_mock_requests(admin_endpoint)
local change_step = apply_step(admin_endpoint, "change")
harness.assert_eq(
    change_step.changes.emails.updated[1],
    "email-002",
    "change step updated id"
)
run_measured_delta(client, account.account_id, "label-only add step")
record_gmail_requests(assert_delta_path(admin_endpoint, "label-only add step"))
local after_change = query_state(client, account.account_id)
local status_update = message_by_id(after_change, "email-002")
harness.assert(status_update ~= nil, "email-002 missing after label-only add")
harness.assert(status_update.is_read, "email-002 did not import read state")
harness.assert(status_update.is_starred, "email-002 did not import starred state")
local thread_after_change, thread_after_change_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = status_update.thread_id,
})
harness.assert(thread_after_change_err == nil, "TestThreadRead after label-only add failed")
harness.assert(thread_after_change.exists, "thread missing after label-only add")
harness.assert(thread_after_change.is_starred, "thread did not import STARRED as state")

harness.clear_mock_requests(admin_endpoint)
local delete_step = apply_step(admin_endpoint, "delete")
harness.assert_eq(
    delete_step.changes.emails.destroyed[1],
    "email-001",
    "delete step destroyed id"
)
run_measured_delta(client, account.account_id, "delete step")
record_gmail_requests(assert_delta_path(admin_endpoint, "delete step"))
local after_delete = query_state(client, account.account_id)
harness.assert_eq(after_delete.message_count, 2, "message count after delete")
harness.assert(message_by_id(after_delete, "email-001") == nil, "email-001 survived delete")

harness.clear_mock_requests(admin_endpoint)
local move_step = apply_step(admin_endpoint, "move")
harness.assert_eq(
    move_step.changes.emails.moved[1],
    "email-002",
    "move step moved id"
)
run_measured_delta(client, account.account_id, "label-only remove step")
record_gmail_requests(assert_delta_path(admin_endpoint, "label-only remove step"))
local after_move = query_state(client, account.account_id)
local moved = message_by_id(after_move, "email-002")
harness.assert(moved ~= nil, "email-002 missing after label-only remove")
local thread_after_move, thread_after_move_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = moved.thread_id,
})
harness.assert(thread_after_move_err == nil, "TestThreadRead after label-only remove failed")
assert_has_value(
    thread_after_move.label_ids,
    "archive",
    "thread labels did not include archive after remove"
)
assert_lacks_value(
    thread_after_move.label_ids,
    "INBOX",
    "thread labels still included INBOX after remove"
)

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
    message_count = after_move.message_count,
    provider_requests = summary_provider_requests,
    gmail_history_requests = summary_history,
    gmail_message_get_requests = summary_message_get,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
