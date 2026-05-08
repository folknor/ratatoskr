-- description: JMAP initial sync imports the small fixture
-- expected: pass
-- fixture: jmap-small
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
harness.assert(state.thread_count >= 1, "thread count")
harness.assert(state.label_count >= 2, "label count")
harness.assert(subject_seen(state.messages, "Hello"), "missing Hello")
harness.assert(subject_seen(state.messages, "Re: Hello"), "missing Re: Hello")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
