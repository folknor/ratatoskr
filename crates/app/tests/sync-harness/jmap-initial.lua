-- description: JMAP initial sync imports the small fixture
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s

local function subject_seen(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return true
        end
    end
    return false
end

local function label_by_id(state, id)
    for _, label in ipairs(state.labels) do
        if label.id == id then
            return label
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-initial@example.test",
    display_name = "Sync JMAP",
    account_name = "Sync JMAP",
    provider = "jmap",
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
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "message count")
harness.assert(state.thread_count >= 1, "thread count")
-- Post labels-unification split: JMAP mailboxes land in `folders`; the only
-- provider-derived `labels` row this fixture produces is the custom keyword
-- on email-001. Assert THAT row - a bare label_count is satisfiable by the
-- harness seed alone.
harness.assert(label_by_id(state, "kw:project") ~= nil, "kw:project label missing")
harness.assert(subject_seen(state.messages, "Hello"), "missing Hello")
harness.assert(subject_seen(state.messages, "Re: Hello"), "missing Re: Hello")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local mailbox_get_requests = harness.request_count(requests, "jmap", "Mailbox/get")
local email_query_requests = harness.request_count(requests, "jmap", "Email/query")
local email_get_requests = harness.request_count(requests, "jmap", "Email/get")
harness.assert(mailbox_get_requests >= 1, "JMAP sync did not call Mailbox/get")
harness.assert(email_query_requests >= 1, "JMAP sync did not call Email/query")
harness.assert(email_get_requests >= 1, "JMAP sync did not call Email/get")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    thread_count = state.thread_count,
    provider_requests = #requests,
    jmap_mailbox_get_requests = mailbox_get_requests,
    jmap_email_query_requests = email_query_requests,
    jmap_email_get_requests = email_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
