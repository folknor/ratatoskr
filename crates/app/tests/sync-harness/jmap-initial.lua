-- description: JMAP initial sync imports the small fixture
-- expected: pass
-- fixture: jmap-small
-- protocol: jmap
-- ceiling: 120s

local function wait_for_sync_completed(queue, run_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "SyncCompleted" then
            if event.run_id == run_id then
                return event
            end
        end
    end
    return nil
end

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

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-initial@example.test",
    display_name = "Sync JMAP",
    account_name = "Sync JMAP",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local started, start_err = client:request("TestStartSync", {
    account_id = account.account_id,
})
harness.assert(start_err == nil, "TestStartSync failed")
harness.assert(started.run_id ~= nil, "sync run id missing")

local completed = wait_for_sync_completed(queue, started.run_id, 30)
harness.assert(completed ~= nil, "missing sync.completed")
harness.assert_eq(completed.result, "completed", "sync result")

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
