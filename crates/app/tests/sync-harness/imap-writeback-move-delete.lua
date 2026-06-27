-- description: IMAP move/delete action writeback persists across follow-up sync
-- expected: pass
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
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

local function read_thread(client, account_id, thread_id, label)
    local thread, err = client:request("TestThreadRead", {
        account_id = account_id,
        thread_id = thread_id,
    })
    harness.assert(err == nil, label .. " TestThreadRead failed")
    return thread
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

local function execute_action(client, queue, account_id, thread_id, operation, fields)
    local op = {
        account_id = account_id,
        thread_id = thread_id,
        operation = operation,
    }
    for key, value in pairs(fields or {}) do
        op[key] = value
    end
    local ack, ack_err = client:request("ActionExecutePlan", {
        operations = { [1] = op },
    })
    harness.assert(ack_err == nil, operation .. " action.execute_plan failed")
    harness.assert(ack.journaled, operation .. " plan was not journaled")

    local completed = wait_for_action_completed(queue, ack.plan_id, 15)
    harness.assert(completed ~= nil, operation .. " missing action.completed")
    harness.assert_eq(completed.summary_total, 1, operation .. " summary total")
    harness.assert_eq(completed.summary_remote_failed, 0, operation .. " remote failures")
    harness.assert_eq(completed.summary_conflicts, 0, operation .. " conflicts")
    harness.assert(
        completed.summary_remote_succeeded >= 1,
        operation .. " did not report remote success"
    )
    return completed
end

-- This gate verifies move/delete by SERVER ROUND-TRIP rather than by matching
-- IMAP wire ops. bifrost performs moves via RFC 6851 UID MOVE (atomic - no
-- separate UID COPY + UID STORE + EXPUNGE), so a wire-op needle would be both
-- brittle and wrong. Instead we apply the action, then resync from saehrimnir
-- and assert the server-reflected folder placement (destination present /
-- source gone / message gone). The flag-writeback gate covers UID STORE.

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")

local dir = harness.data_dir("sync_imap_writeback_move_delete")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-writeback-move@example.test",
    display_name = "Sync IMAP Move Delete",
    account_name = "Sync IMAP Move Delete",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial_sync, initial_sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_sync_err == nil, "initial start_sync failed")
harness.assert_eq(
    initial_sync.result,
    "completed",
    initial_sync.error or "initial sync result"
)

local initial, initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(initial_err == nil, "initial TestQueryDbState failed")
harness.assert_eq(initial.message_count, 2, "initial message count")
harness.assert_eq(initial.thread_count, 1, "initial thread count")

local hello = message_by_subject(initial.messages, "Hello")
harness.assert(hello ~= nil, "missing Hello")

local initial_thread = read_thread(client, account.account_id, hello.thread_id, "initial")
harness.assert(initial_thread.exists, "initial thread missing")
assert_has_label(initial_thread, "INBOX", "initial thread")
assert_lacks_label(initial_thread, "archive", "initial thread")

execute_action(
    client,
    queue,
    account.account_id,
    hello.thread_id,
    "MoveToFolder",
    { dest = "archive", source = "INBOX" }
)

local after_move_thread = read_thread(client, account.account_id, hello.thread_id, "after move")
harness.assert(after_move_thread.exists, "thread missing after move")
assert_has_label(after_move_thread, "archive", "after move")
assert_lacks_label(after_move_thread, "INBOX", "after move")

local move_resync, move_resync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(move_resync_err == nil, "post-MoveToFolder sync failed")
harness.assert_eq(move_resync.result, "completed", move_resync.error or "post-MoveToFolder sync")

local after_move_resync, after_move_resync_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_move_resync_err == nil, "TestQueryDbState after move resync failed")
harness.assert_eq(after_move_resync.message_count, 2, "message count after move resync")
harness.assert_eq(after_move_resync.thread_count, 1, "thread count after move resync")

local after_move_resync_thread =
    read_thread(client, account.account_id, hello.thread_id, "after move resync")
harness.assert(after_move_resync_thread.exists, "thread missing after move resync")
assert_has_label(after_move_resync_thread, "archive", "after move resync")
assert_lacks_label(after_move_resync_thread, "INBOX", "after move resync")

execute_action(
    client,
    queue,
    account.account_id,
    hello.thread_id,
    "PermanentDelete"
)

local after_delete_thread = read_thread(client, account.account_id, hello.thread_id, "after delete")
harness.assert(not after_delete_thread.exists, "thread still exists after delete")

local after_delete, after_delete_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_delete_err == nil, "TestQueryDbState after delete failed")
harness.assert_eq(after_delete.message_count, 0, "message count after delete")
harness.assert_eq(after_delete.thread_count, 0, "thread count after delete")

local delete_resync, delete_resync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(delete_resync_err == nil, "post-PermanentDelete sync failed")
harness.assert_eq(
    delete_resync.result,
    "completed",
    delete_resync.error or "post-PermanentDelete sync"
)

local after_delete_resync, after_delete_resync_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_delete_resync_err == nil, "TestQueryDbState after delete resync failed")
harness.assert_eq(after_delete_resync.message_count, 0, "message count after delete resync")
harness.assert_eq(after_delete_resync.thread_count, 0, "thread count after delete resync")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
