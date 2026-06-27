-- description: JMAP client send dispatches via EmailSubmission and the sent message round-trips into Sent
-- expected: pass
-- fixture: send-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- Per-provider send-writeback gate (B5-GATES). Drives the real send action
-- (ActionSend -> resident SyncEngine engine.send_message -> JMAP Email/set +
-- EmailSubmission/set against saehrimnir), then resyncs and asserts the sent
-- message round-trips into the server's Sent mailbox.
--
-- Verification is two-layered, mirroring the B4a action-writeback gates:
--   (1) the action.completed summary shows the send dispatched REMOTELY
--       (remote_succeeded >= 1, remote_failed == 0, local_only == 0) - a
--       local-only degrade that never reached the provider is the exact
--       regression this guards;
--   (2) a fresh resync brings the sent message back filed under SENT, proving
--       the EmailSubmission actually propagated server-side rather than the
--       gate trusting the local completion alone.

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

local function wait_for_action_completed(queue, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" and event.plan_id == plan_id then
            return event
        end
    end
    return nil
end

local dir = harness.data_dir("sync_jmap_send_writeback")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Sync JMAP Send",
    account_name = "Sync JMAP Send",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- Compose-send a fresh message with a unique subject.
local send_id = harness.uuid()
local subject = "jmap send roundtrip " .. send_id
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    message = {
        draft_id = "draft-" .. send_id,
        from = "test@example.com",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = subject,
        body_html = "<p>jmap send roundtrip</p>",
        body_text = "jmap send roundtrip",
    },
    attachments = {},
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "send was not journaled")
harness.assert_eq(ack.send_id, send_id, "send ack id")

local completed = wait_for_action_completed(queue, send_id, 30)
harness.assert(completed ~= nil, "missing action.completed")
harness.assert_eq(completed.summary_total, 1, "send completion total")
harness.assert_eq(completed.summary_remote_failed, 0, "send remote failures")
harness.assert_eq(completed.summary_local_only, 0, "send degraded to local-only")
harness.assert_eq(completed.summary_conflicts, 0, "send conflicts")
harness.assert(
    completed.summary_remote_succeeded >= 1,
    "send did not report a remote success"
)

-- Round-trip: resync and assert the sent message now lives on the server in the
-- Sent mailbox. A local-only degrade would leave nothing for the server to hand
-- back, so the message's presence under SENT after a fresh sync proves the
-- EmailSubmission propagated.
local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-send resync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-send resync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local sent = message_by_subject(state.messages, subject)
harness.assert(sent ~= nil, "sent message did not round-trip from the server")

local thread, thread_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = sent.thread_id,
})
harness.assert(thread_err == nil, "TestThreadRead failed")
harness.assert(
    has_label(thread.label_ids, "SENT"),
    "sent message did not land under SENT on the server"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
