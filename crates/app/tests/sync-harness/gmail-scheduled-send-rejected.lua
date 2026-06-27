-- description: Gmail scheduled-send is rejected at the capability gate with no wire call
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s
--
-- B5c capability-gate gate (read the B5 spec section 4.4). Gmail advertises as
-- a PIM-rich provider, but its REST API has NO server-side delayed-send lever
-- (web-UI only), so bifrost reports `capabilities().pim_methods.scheduled_send
-- = false` for Gmail. ratatoskr's `send_scheduled` reads that flag as the
-- single source of truth and rejects an `action.send` carrying `scheduled_at`
-- BEFORE any provider round-trip - the capability flag, never a hardcoded
-- provider allowlist, decides.
--
-- This gate issues a scheduled `action.send` against a Gmail account and
-- asserts the journaled send finalizes FAILED (the gate rejected it) rather
-- than dispatching. It needs no saehrimnir send surface: the rejection fires in
-- ratatoskr before the engine would touch the wire.

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

local dir = harness.data_dir("sync_gmail_scheduled_send_rejected")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "scheduled-gmail@example.test",
    display_name = "Scheduled Gmail",
    account_name = "Scheduled Gmail",
    provider = "gmail",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- Attach the resident engine slot so `send_scheduled` can read the account's
-- capabilities (it resolves a resident_action_account, which keep-attaches).
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

-- A far-future absolute instant (2100-01-01). The value is irrelevant: the
-- capability gate rejects before `validate_scheduled` ever runs.
local scheduled_at = 4102444800
local send_id = harness.uuid()
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    scheduled_at = scheduled_at,
    message = {
        draft_id = "draft-scheduled-gmail",
        from = "scheduled-gmail@example.test",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = "scheduled gmail",
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

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
