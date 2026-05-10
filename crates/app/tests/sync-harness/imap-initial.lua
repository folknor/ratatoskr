-- description: IMAP initial sync imports the small fixture with flags
-- expected: pass
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-initial@example.test",
    display_name = "Sync IMAP",
    account_name = "Sync IMAP",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "message count")
harness.assert_eq(state.unread_message_count, 1, "unread message count")
harness.assert(state.thread_count >= 1, "thread count")
harness.assert(state.label_count >= 2, "label count")

local synced_account = account_by_id(state, account.account_id)
harness.assert(synced_account ~= nil, "account missing after sync")
harness.assert(
    synced_account.initial_sync_completed,
    "initial sync did not mark account completed"
)

local hello = message_by_subject(state.messages, "Hello")
harness.assert(hello ~= nil, "missing Hello")
harness.assert(hello.is_read, "Hello should import as read from $seen")
harness.assert(not hello.is_starred, "Hello should not import as starred")

local reply = message_by_subject(state.messages, "Re: Hello")
harness.assert(reply ~= nil, "missing Re: Hello")
harness.assert(not reply.is_read, "Re: Hello should import as unread")
harness.assert(reply.is_starred, "Re: Hello should import as starred from $flagged")

local requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(requests, "imap", "LIST") >= 1,
    "IMAP sync did not list folders"
)
harness.assert(
    harness.request_count(requests, "imap", "UID SEARCH") >= 1,
    "IMAP sync did not search UIDs"
)
harness.assert(
    harness.request_count(requests, "imap", "UID FETCH") >= 1,
    "IMAP sync did not fetch messages"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
