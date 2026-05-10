-- description: JMAP delta imports a fixture mutation written through IMAP
-- expected: pass
-- fixture: jmap-incremental.lua
-- protocol: jmap
-- ceiling: 120s
-- measured: JMAP convergence sync after the IMAP action mutates shared fixture state

local function message_by_subject(state, subject)
    for _, message in ipairs(state.messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function has_label(labels, expected)
    for _, label in ipairs(labels) do
        if label == expected then
            return true
        end
    end
    return false
end

local function assert_has_label(thread, expected, label)
    harness.assert(has_label(thread.label_ids, expected), label .. " missing " .. expected)
end

local function assert_lacks_label(thread, expected, label)
    harness.assert(not has_label(thread.label_ids, expected), label .. " still has " .. expected)
end

local function query_state(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, label .. " TestQueryDbState failed")
    return state
end

local function read_thread(client, account_id, thread_id, label)
    local thread, err = client:request("TestThreadRead", {
        account_id = account_id,
        thread_id = thread_id,
    })
    harness.assert(err == nil, label .. " TestThreadRead failed")
    return thread
end

local function run_sync(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local measured_sync_count = 0

local function run_measured_sync(client, account_id, label)
    harness.marker("SYNC_START")
    run_sync(client, account_id, label)
    harness.marker("SYNC_END")
    measured_sync_count = measured_sync_count + 1
end

local function wait_for_action_completed(queue, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" then
            if event.plan_id == plan_id then
                return event
            end
        end
    end
    return nil
end

local function execute_move_to_archive(client, queue, account_id, thread_id)
    local ack, ack_err = client:request("ActionExecutePlan", {
        operations = {
            {
                account_id = account_id,
                thread_id = thread_id,
                operation = "MoveToFolder",
                dest = "archive",
                source = "INBOX",
            },
        },
    })
    harness.assert(ack_err == nil, "MoveToFolder action.execute_plan failed")
    harness.assert(ack.journaled, "MoveToFolder plan was not journaled")

    local completed = wait_for_action_completed(queue, ack.plan_id, 15)
    harness.assert(completed ~= nil, "MoveToFolder missing action.completed")
    harness.assert_eq(completed.summary_total, 1, "MoveToFolder summary total")
    harness.assert_eq(completed.summary_remote_failed, 0, "MoveToFolder remote failures")
    harness.assert_eq(completed.summary_conflicts, 0, "MoveToFolder conflicts")
    harness.assert(
        completed.summary_remote_succeeded >= 1,
        "MoveToFolder did not report remote success"
    )
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_jmap_shared_state")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local imap_account, imap_account_err = client:request("TestSeedAccount", {
    email = "sync-cross-imap@example.test",
    display_name = "Sync Cross IMAP",
    account_name = "Sync Cross IMAP",
    provider = "imap",
})
harness.assert(imap_account_err == nil, "TestSeedAccount IMAP failed")

local jmap_account, jmap_account_err = client:request("TestSeedAccount", {
    email = "sync-cross-jmap@example.test",
    display_name = "Sync Cross JMAP",
    account_name = "Sync Cross JMAP",
    provider = "jmap",
})
harness.assert(jmap_account_err == nil, "TestSeedAccount JMAP failed")

run_sync(client, imap_account.account_id, "initial IMAP")
run_sync(client, jmap_account.account_id, "initial JMAP")

local imap_initial = query_state(client, imap_account.account_id, "initial IMAP")
local imap_status = message_by_subject(imap_initial, "Status update")
harness.assert(imap_status ~= nil, "IMAP Status update missing before move")

local jmap_initial = query_state(client, jmap_account.account_id, "initial JMAP")
harness.assert_eq(jmap_initial.message_count, 2, "initial JMAP message count")
local jmap_status = message_by_subject(jmap_initial, "Status update")
harness.assert(jmap_status ~= nil, "JMAP Status update missing before move")
local jmap_initial_thread =
    read_thread(client, jmap_account.account_id, jmap_status.thread_id, "initial JMAP")
assert_has_label(jmap_initial_thread, "INBOX", "initial JMAP thread")
assert_lacks_label(jmap_initial_thread, "archive", "initial JMAP thread")

execute_move_to_archive(client, queue, imap_account.account_id, imap_status.thread_id)

local imap_after_move =
    read_thread(client, imap_account.account_id, imap_status.thread_id, "IMAP after move")
assert_has_label(imap_after_move, "archive", "IMAP moved thread")
assert_lacks_label(imap_after_move, "INBOX", "IMAP moved thread")

harness.clear_mock_requests(admin_endpoint)

run_measured_sync(client, jmap_account.account_id, "JMAP delta after IMAP move")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local email_changes = harness.request_count(requests, "jmap", "Email/changes")
local email_get = harness.request_count(requests, "jmap", "Email/get")
local email_query = harness.request_count(requests, "jmap", "Email/query")
harness.assert(
    email_changes >= 1,
    "JMAP delta did not call Email/changes"
)
harness.assert(
    email_get >= 1,
    "JMAP delta did not fetch changed email ids"
)
harness.assert_eq(
    email_query,
    0,
    "JMAP delta unexpectedly ran Email/query"
)

local jmap_after_move = query_state(client, jmap_account.account_id, "JMAP after IMAP move")
harness.assert_eq(jmap_after_move.message_count, 2, "JMAP message count after IMAP move")
local jmap_moved = message_by_subject(jmap_after_move, "Status update")
harness.assert(jmap_moved ~= nil, "JMAP Status update missing after IMAP move")
local jmap_moved_thread =
    read_thread(client, jmap_account.account_id, jmap_moved.thread_id, "JMAP after IMAP move")
assert_has_label(jmap_moved_thread, "archive", "JMAP moved thread")
assert_lacks_label(jmap_moved_thread, "INBOX", "JMAP moved thread")

harness.write_summary({
    correct = 1,
    measured_syncs = measured_sync_count,
    provider_requests = #requests,
    message_count = jmap_after_move.message_count,
    email_changes = email_changes,
    email_get = email_get,
    email_query = email_query,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
