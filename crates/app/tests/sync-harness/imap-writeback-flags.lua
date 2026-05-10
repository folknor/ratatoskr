-- description: IMAP read/star action writeback persists across follow-up sync
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

local function assert_messages_read(state, expected, label)
    for _, message in ipairs(state.messages) do
        harness.assert_eq(message.is_read, expected, label .. " read flag")
    end
end

local function assert_messages_starred(state, expected, label)
    for _, message in ipairs(state.messages) do
        harness.assert_eq(message.is_starred, expected, label .. " starred flag")
    end
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_writeback_flags")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-writeback@example.test",
    display_name = "Sync IMAP Writeback",
    account_name = "Sync IMAP Writeback",
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
harness.assert_eq(initial.unread_message_count, 1, "initial unread count")

local hello = message_by_subject(initial.messages, "Hello")
harness.assert(hello ~= nil, "missing Hello")
local reply = message_by_subject(initial.messages, "Re: Hello")
harness.assert(reply ~= nil, "missing Re: Hello")
harness.assert_eq(hello.thread_id, reply.thread_id, "fixture messages should share thread")
harness.assert(hello.is_read, "Hello should start read")
harness.assert(not reply.is_read, "Re: Hello should start unread")
harness.assert(not hello.is_starred, "Hello should start unstarred")
harness.assert(reply.is_starred, "Re: Hello should start starred")

harness.clear_mock_requests(admin_endpoint)

execute_action(
    client,
    queue,
    account.account_id,
    hello.thread_id,
    "SetRead",
    { to = true }
)

local read_requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(read_requests, "imap", "UID STORE") >= 1,
    "SetRead did not issue UID STORE"
)

local after_read, after_read_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_read_err == nil, "TestQueryDbState after SetRead failed")
harness.assert_eq(after_read.unread_message_count, 0, "unread count after SetRead")
assert_messages_read(after_read, true, "after SetRead")

harness.clear_mock_requests(admin_endpoint)

local read_resync, read_resync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(read_resync_err == nil, "post-SetRead sync failed")
harness.assert_eq(read_resync.result, "completed", read_resync.error or "post-SetRead sync")

local after_read_resync, after_read_resync_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_read_resync_err == nil, "TestQueryDbState after SetRead resync failed")
harness.assert_eq(after_read_resync.unread_message_count, 0, "unread count after SetRead resync")
assert_messages_read(after_read_resync, true, "after SetRead resync")

harness.clear_mock_requests(admin_endpoint)

execute_action(
    client,
    queue,
    account.account_id,
    hello.thread_id,
    "SetStarred",
    { to = false }
)

local star_requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(star_requests, "imap", "UID STORE") >= 1,
    "SetStarred did not issue UID STORE"
)

local after_star, after_star_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_star_err == nil, "TestQueryDbState after SetStarred failed")
assert_messages_starred(after_star, false, "after SetStarred")

harness.clear_mock_requests(admin_endpoint)

local star_resync, star_resync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(star_resync_err == nil, "post-SetStarred sync failed")
harness.assert_eq(star_resync.result, "completed", star_resync.error or "post-SetStarred sync")

local after_star_resync, after_star_resync_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(
    after_star_resync_err == nil,
    "TestQueryDbState after SetStarred resync failed"
)
assert_messages_starred(after_star_resync, false, "after SetStarred resync")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
