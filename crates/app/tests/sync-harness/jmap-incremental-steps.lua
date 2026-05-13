-- description: JMAP scripted fixture steps are imported by incremental sync
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: jmap-incremental.lua
-- protocol: jmap
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
        harness.request_count(requests, "jmap", "Email/changes") >= 1,
        label .. " did not call Email/changes"
    )
    harness.assert(
        harness.request_count(requests, "jmap", "Email/get") >= 1,
        label .. " did not fetch changed email ids"
    )
    harness.assert_eq(
        harness.request_count(requests, "jmap", "Email/query"),
        0,
        label .. " unexpectedly ran Email/query"
    )
    return requests
end

local summary_provider_requests = 0
local summary_email_changes = 0
local summary_email_get = 0
local summary_email_query = 0

local function record_jmap_requests(requests)
    summary_provider_requests = summary_provider_requests + #requests
    summary_email_changes =
        summary_email_changes + harness.request_count(requests, "jmap", "Email/changes")
    summary_email_get =
        summary_email_get + harness.request_count(requests, "jmap", "Email/get")
    summary_email_query =
        summary_email_query + harness.request_count(requests, "jmap", "Email/query")
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("sync_jmap_incremental_steps")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-incremental@example.test",
    display_name = "Sync JMAP Incremental",
    account_name = "Sync JMAP Incremental",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_delta(client, account.account_id, "initial")
local initial_snapshot = fixture_snapshot(jmap_endpoint)
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

harness.clear_mock_requests(jmap_endpoint)
local new_step = apply_step(jmap_endpoint, "new")
harness.assert_eq(
    new_step.changes.emails.created[1],
    "email-003",
    "new step created id"
)
local remote_after_new = fixture_snapshot(jmap_endpoint)
local remote_lunch = fixture_email_by_id(remote_after_new, "email-003")
harness.assert(remote_lunch ~= nil, "remote snapshot missing email-003")
harness.assert_eq(remote_lunch.subject, "Lunch?", "remote new email subject")
assert_has_value(
    remote_lunch.mailbox_ids,
    "mb-inbox",
    "remote new email did not land in inbox"
)
run_measured_delta(client, account.account_id, "new step")
record_jmap_requests(assert_delta_path(jmap_endpoint, "new step"))
local after_new = query_state(client, account.account_id)
harness.assert_eq(after_new.message_count, 3, "message count after new")
local lunch = message_by_id(after_new, "email-003")
harness.assert(lunch ~= nil, "new email missing after delta")
harness.assert_eq(lunch.subject, "Lunch?", "new email subject")

harness.clear_mock_requests(jmap_endpoint)
local change_step = apply_step(jmap_endpoint, "change")
harness.assert_eq(
    change_step.changes.emails.updated[1],
    "email-002",
    "change step updated id"
)
local remote_after_change = fixture_snapshot(jmap_endpoint)
local remote_status_update = fixture_email_by_id(remote_after_change, "email-002")
harness.assert(remote_status_update ~= nil, "remote snapshot missing email-002 after change")
assert_has_value(
    remote_status_update.keywords,
    "$seen",
    "remote email-002 did not gain $seen"
)
assert_has_value(
    remote_status_update.keywords,
    "$flagged",
    "remote email-002 did not gain $flagged"
)
run_measured_delta(client, account.account_id, "change step")
record_jmap_requests(assert_delta_path(jmap_endpoint, "change step"))
local after_change = query_state(client, account.account_id)
local status_update = message_by_id(after_change, "email-002")
harness.assert(status_update ~= nil, "email-002 missing after change")
harness.assert(status_update.is_read, "email-002 did not import $seen")
harness.assert(status_update.is_starred, "email-002 did not import $flagged")
local thread_after_change, thread_after_change_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = status_update.thread_id,
})
harness.assert(thread_after_change_err == nil, "TestThreadRead after change failed")
assert_has_value(
    thread_after_change.label_ids,
    "STARRED",
    "thread labels did not include STARRED after change"
)

harness.clear_mock_requests(jmap_endpoint)
local delete_step = apply_step(jmap_endpoint, "delete")
harness.assert_eq(
    delete_step.changes.emails.destroyed[1],
    "email-001",
    "delete step destroyed id"
)
local remote_after_delete = fixture_snapshot(jmap_endpoint)
harness.assert(
    fixture_email_by_id(remote_after_delete, "email-001") == nil,
    "remote snapshot retained email-001 after delete"
)
run_measured_delta(client, account.account_id, "delete step")
local delete_requests = harness.mock_requests(jmap_endpoint, { stable = true })
harness.assert(
    harness.request_count(delete_requests, "jmap", "Email/changes") >= 1,
    "delete step did not call Email/changes"
)
harness.assert_eq(
    harness.request_count(delete_requests, "jmap", "Email/query"),
    0,
    "delete step unexpectedly ran Email/query"
)
record_jmap_requests(delete_requests)
local after_delete = query_state(client, account.account_id)
harness.assert_eq(after_delete.message_count, 2, "message count after delete")
harness.assert(message_by_id(after_delete, "email-001") == nil, "email-001 survived delete")

harness.clear_mock_requests(jmap_endpoint)
local move_step = apply_step(jmap_endpoint, "move")
harness.assert_eq(
    move_step.changes.emails.moved[1],
    "email-002",
    "move step moved id"
)
local remote_after_move = fixture_snapshot(jmap_endpoint)
local remote_moved = fixture_email_by_id(remote_after_move, "email-002")
harness.assert(remote_moved ~= nil, "remote snapshot missing email-002 after move")
assert_has_value(
    remote_moved.mailbox_ids,
    "mb-archive",
    "remote email-002 did not move to archive"
)
assert_lacks_value(
    remote_moved.mailbox_ids,
    "mb-inbox",
    "remote email-002 stayed in inbox after move"
)
run_measured_delta(client, account.account_id, "move step")
record_jmap_requests(assert_delta_path(jmap_endpoint, "move step"))
local after_move = query_state(client, account.account_id)
local moved = message_by_id(after_move, "email-002")
harness.assert(moved ~= nil, "email-002 missing after move")
local thread_after_move, thread_after_move_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = moved.thread_id,
})
harness.assert(thread_after_move_err == nil, "TestThreadRead after move failed")
assert_has_value(
    thread_after_move.label_ids,
    "archive",
    "thread labels did not include archive after move"
)
assert_lacks_value(
    thread_after_move.label_ids,
    "INBOX",
    "thread labels still included INBOX after move"
)

local end_response = harness.http_json({
    method = "POST",
    url = harness.join_url(jmap_endpoint, "test/fixture/step"),
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
    email_changes_requests = summary_email_changes,
    email_get_requests = summary_email_get,
    email_query_requests = summary_email_query,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
