-- description: JMAP scheduled send is accepted (FUTURERELEASE) and round-trips into Sent
-- expected: pass
-- fixture: send-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- Scheduled-send round-trip gate (B5-GATES). The capable-provider counterpart
-- to gmail-/imap-scheduled-send-rejected.lua: JMAP advertises a non-zero
-- maxDelayedSend, so bifrost reports pim_methods.scheduled_send = true and
-- ratatoskr's send_scheduled passes the capability gate and dispatches the send
-- with a FUTURERELEASE (holduntil) envelope parameter rather than rejecting it.
--
-- Verification mirrors jmap-send-writeback: the action.completed summary shows a
-- clean REMOTE dispatch (so a scheduled send on a CAPABLE provider is NOT
-- rejected - the inverse of the *-scheduled-send-rejected gates), and a fresh
-- resync brings the submitted message back under SENT.

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

local dir = harness.data_dir("sync_jmap_scheduled_send")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Sync JMAP Scheduled",
    account_name = "Sync JMAP Scheduled",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- A real future instant (wall-clock + 10 min), strictly in the future and well
-- within the mock's advertised maxDelayedSend window (1 year). validate_scheduled
-- accepts it; the gate then proves the send DISPATCHED rather than being
-- rejected at the capability gate.
local scheduled_at = math.floor(harness.wall_ms() / 1000) + 600
local send_id = harness.uuid()
local subject = "jmap scheduled send " .. send_id
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    scheduled_at = scheduled_at,
    message = {
        draft_id = "draft-" .. send_id,
        from = "test@example.com",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = subject,
        body_html = "<p>jmap scheduled send</p>",
        body_text = "jmap scheduled send",
    },
    attachments = {},
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "scheduled send was not journaled")
harness.assert_eq(ack.send_id, send_id, "send ack id")

local completed = wait_for_action_completed(queue, send_id, 30)
harness.assert(completed ~= nil, "missing action.completed")
harness.assert_eq(completed.summary_total, 1, "scheduled send completion total")
harness.assert_eq(completed.summary_remote_failed, 0, "scheduled send remote failures")
harness.assert_eq(completed.summary_local_only, 0, "scheduled send degraded to local-only")
harness.assert_eq(completed.summary_conflicts, 0, "scheduled send conflicts")
harness.assert(
    completed.summary_remote_succeeded >= 1,
    "scheduled send was not accepted on a scheduled-send-capable provider"
)

local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-send resync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-send resync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local sent = message_by_subject(state.messages, subject)
harness.assert(sent ~= nil, "scheduled message did not round-trip from the server")

local thread, thread_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = sent.thread_id,
})
harness.assert(thread_err == nil, "TestThreadRead failed")
harness.assert(
    has_label(thread.label_ids, "SENT"),
    "scheduled message did not land under SENT on the server"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
