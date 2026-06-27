-- description: JMAP action writeback dispatches remotely and survives a server round-trip
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- Per-provider action-writeback gate for the bifrost engine dispatch cut
-- (B4a; read the B4a landing commit). Drives the real action
-- pipeline (ActionExecutePlan -> resident SyncEngine mutation passthrough)
-- against saehrimnir.
--
-- Verification is by SERVER ROUND-TRIP, not provider-wire-op string matching.
-- For each action we (1) assert the action.completed summary shows the op
-- dispatched REMOTELY (remote_succeeded >= 1, remote_failed == 0, conflicts == 0
-- and crucially local_only == 0 - a local-only degrade that never reached the
-- provider lands on local_only, so this alone separates a real remote dispatch
-- from a silent fallback), then (2) resync the account from the mock and assert
-- the SERVER-side state now reflects the mutation. The round-trip proves
-- propagation without coupling the gate to JMAP's internal Email/set wire shape.

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
    -- A local-only degrade (dispatch never reached the provider) is the exact
    -- regression this gate guards: it must be zero, and the op must report a
    -- real remote success.
    harness.assert_eq(completed.summary_local_only, 0, operation .. " degraded to local-only")
    harness.assert(
        completed.summary_remote_succeeded >= 1,
        operation .. " did not report remote success"
    )
    return completed
end

local function query(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, "TestQueryDbState " .. label .. " failed")
    return state
end

local function resync(client, account_id, label)
    local result, err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(err == nil, label .. " resync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " resync"))
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")

local dir = harness.data_dir("sync_jmap_action_writeback")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-writeback@example.test",
    display_name = "Sync JMAP Writeback",
    account_name = "Sync JMAP Writeback",
    provider = "jmap",
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

local initial = query(client, account.account_id, "initial")
harness.assert_eq(initial.message_count, 2, "initial message count")
local hello = message_by_subject(initial.messages, "Hello")
harness.assert(hello ~= nil, "missing Hello")

-- SetRead: dispatch remotely, then resync and assert the ACTED thread's
-- messages come back read from the server. The fixture's two messages both
-- live in thread-hello, so we scope the assertion to that thread rather than a
-- whole-account unread count (which other fixture mail could spoil).
execute_action(client, queue, account.account_id, hello.thread_id, "SetRead", { to = true })
resync(client, account.account_id, "SetRead")
harness.assert_eq(
    read_thread(client, account.account_id, hello.thread_id, "after SetRead resync").unread_messages,
    0,
    "thread unread after SetRead resync"
)

-- SetStarred: dispatch remotely, then resync and assert the acted thread comes
-- back starred from the server.
execute_action(client, queue, account.account_id, hello.thread_id, "SetStarred", { to = true })
resync(client, account.account_id, "SetStarred")
harness.assert(
    read_thread(client, account.account_id, hello.thread_id, "after SetStarred resync").is_starred,
    "thread starred after SetStarred resync"
)

-- Archive: dispatch remotely, then resync and assert the thread left the inbox
-- on the server.
execute_action(client, queue, account.account_id, hello.thread_id, "Archive")
resync(client, account.account_id, "Archive")
local archived = read_thread(client, account.account_id, hello.thread_id, "after Archive resync")
harness.assert(archived.exists, "thread missing after Archive resync")
harness.assert(
    not has_label(archived.label_ids, "INBOX"),
    "thread still in inbox after Archive resync"
)

-- PermanentDelete: dispatch remotely, then resync and assert the acted thread
-- is gone from the server.
execute_action(client, queue, account.account_id, hello.thread_id, "PermanentDelete")
resync(client, account.account_id, "PermanentDelete")
harness.assert(
    not read_thread(client, account.account_id, hello.thread_id, "after delete resync").exists,
    "thread gone after delete resync"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
