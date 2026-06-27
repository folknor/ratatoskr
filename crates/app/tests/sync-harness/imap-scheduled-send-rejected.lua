-- description: IMAP scheduled-send is rejected at the capability gate with no wire call
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s
--
-- B5c capability-gate gate (read the B5 spec section 4.4). FUTURERELEASE is a
-- per-connection SMTP EHLO truth ratatoskr treats as statically absent, so
-- bifrost reports `capabilities().pim_methods.scheduled_send = false` for IMAP
-- accounts. ratatoskr's `send_scheduled` reads that flag and rejects an
-- `action.send` carrying `scheduled_at` before any SMTP submission - the
-- unwired local-delegation scaffold stays unwired for incapable providers.
--
-- This gate issues a scheduled `action.send` against an IMAP account and
-- asserts the journaled send finalizes FAILED (the gate rejected it). No SMTP
-- submission should fire; the rejection is entirely in ratatoskr.

local function smtp_log_url()
    local base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
    harness.assert(base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
    if string.sub(base, -1) == "/" then
        return base .. "test/smtp/submissions"
    end
    return base .. "/test/smtp/submissions"
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

local dir = harness.data_dir("sync_imap_scheduled_send_rejected")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local account, account_err = client:request("TestSeedAccount", {
    email = "scheduled-imap@example.com",
    display_name = "Scheduled IMAP",
    account_name = "Scheduled IMAP",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- Attach the resident engine slot so `send_scheduled` can read capabilities.
local initial_sync, initial_sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_sync_err == nil, "initial start_sync failed")
harness.assert_eq(
    initial_sync.result,
    "completed",
    initial_sync.error or "initial sync result"
)

local queue = client:notifications()

local scheduled_at = 4102444800
local send_id = harness.uuid()
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    scheduled_at = scheduled_at,
    message = {
        draft_id = "draft-scheduled-imap",
        from = "scheduled-imap@example.com",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = "scheduled imap",
        body_html = "<p>later</p>",
        body_text = "later",
    },
    attachments = {},
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "scheduled send was not journaled")

local completed = wait_for_action_completed(queue, send_id, 30)
harness.assert(completed ~= nil, "missing action.completed for scheduled send")
harness.assert_eq(completed.summary_total, 1, "scheduled send summary total")
harness.assert_eq(
    completed.summary_remote_succeeded,
    0,
    "scheduled send must not report a remote success on an incapable provider"
)
harness.assert_eq(
    completed.summary_remote_failed,
    1,
    "scheduled send must be rejected at the capability gate"
)

-- No SMTP submission must have fired: the gate rejects before the engine
-- reaches the submission transport.
local submissions = harness.http_get(log_url)
if submissions ~= nil then
    harness.assert_eq(
        #submissions,
        0,
        "a rejected scheduled send must not produce an SMTP submission"
    )
end

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
