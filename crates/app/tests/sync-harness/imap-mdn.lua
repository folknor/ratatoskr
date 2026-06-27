-- description: reading an MDN-requesting message sends an RFC 8098 read receipt to the sender
-- expected: pass
-- fixture: mdn-small.toml
-- protocol: imap
-- ceiling: 120s
--
-- MDN (read-receipt) round-trip gate (B5-GATES). The fixture's Inbox message
-- carries a Disposition-Notification-To header (via body_raw_bytes), so the
-- consumer sets mdn_requested = 1 at hydrate time. Seeding the account with an
-- "always" read-receipt policy forces the auto-send branch. When the thread is
-- marked read (SetRead), ratatoskr composes an RFC 8098 MDN and sends it via
-- engine.send_raw_message -> the IMAP account's SMTP backend. The gate verifies
-- the MDN reached the server by inspecting the mock's SMTP submission log: a
-- submission FROM the account TO the original sender.

local function smtp_log_url()
    local base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
    harness.assert(base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
    if string.sub(base, -1) == "/" then
        return base .. "test/smtp/submissions"
    end
    return base .. "/test/smtp/submissions"
end

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
        if event ~= nil and event.type == "ActionCompleted" and event.plan_id == plan_id then
            return event
        end
    end
    return nil
end

local function wait_for_submissions(url, count, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local submissions = harness.http_get(url)
        if submissions ~= nil and #submissions == count then
            return submissions
        end
        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("sync_imap_mdn")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local queue = client:notifications()

-- Bind the account to the fixture's primary (account name test@example.com) and
-- force the auto-send branch with an account-scoped "always" policy.
local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Sync IMAP MDN",
    account_name = "Sync IMAP MDN",
    provider = "imap",
    read_receipt_policy = "always",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- Locate the MDN-requesting message that synced into the Inbox.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local message = message_by_subject(state.messages, "Please confirm receipt")
harness.assert(message ~= nil, "MDN-requesting message did not sync")

-- Mark the thread read; the read-receipt follow-up dispatches the MDN.
local ack, ack_err = client:request("ActionExecutePlan", {
    operations = {
        [1] = {
            account_id = account.account_id,
            thread_id = message.thread_id,
            operation = "SetRead",
            to = true,
        },
    },
})
harness.assert(ack_err == nil, "ActionExecutePlan SetRead failed")
harness.assert(ack.journaled, "SetRead was not journaled")

local completed = wait_for_action_completed(queue, ack.plan_id, 30)
harness.assert(completed ~= nil, "missing action.completed for SetRead")

-- The MDN must have been submitted to the original sender.
local submissions = wait_for_submissions(log_url, 1, 30)
harness.assert(submissions ~= nil, "MDN was not submitted to the SMTP relay")
local submission = submissions[1]
harness.assert_eq(submission.from, "<test@example.com>", "MDN MAIL FROM")
harness.assert_eq(
    submission.recipients[1],
    "<sender@example.com>",
    "MDN was not addressed to the requesting sender"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
