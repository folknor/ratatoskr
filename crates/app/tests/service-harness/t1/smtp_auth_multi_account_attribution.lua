-- description: SMTP AUTH binds each compose-send submission to the authenticating account_id
-- fixture: multi-account-small.toml
-- protocol: smtp
-- ceiling: 120s

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

local function submission_by_subject(submissions, subject)
    for _, submission in ipairs(submissions) do
        if submission.parsed ~= nil and submission.parsed.subject == subject then
            return submission
        end
    end
    return nil
end

local dir = harness.data_dir("t1_smtp_auth_multi_account_attribution")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

-- Two accounts with emails matching the multi-account-small fixture
-- principals. Saehrimnir's SMTP AUTH PLAIN handler matches the
-- credential's username (case-insensitive) against fixture
-- `account.name` (the email column) and binds the connection's
-- account_id to that match.
local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "SMTP Primary",
    account_name = "SMTP Primary",
    provider = "imap",
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "SMTP Secondary",
    account_name = "SMTP Secondary",
    provider = "imap",
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

local queue = client:notifications()

local function send_from(account, subject_tag)
    local send_id = harness.uuid()
    local subject = "smtp-attribution " .. subject_tag
    local ack, ack_err = client:request("ActionSend", {
        send_id = send_id,
        from_account_id = account.account_id,
        message = {
            draft_id = "draft-" .. subject_tag,
            from = account.email or "",
            to = { subject_tag .. "-recipient@example.test" },
            cc = {},
            bcc = {},
            subject = subject,
            body_html = "<p>" .. subject_tag .. "</p>",
            body_text = subject_tag .. " plain body",
        },
        attachments = {},
    })
    harness.assert(ack_err == nil, subject_tag .. " action.send failed")
    harness.assert(ack.journaled, subject_tag .. " send was not journaled")

    local completed = wait_for_action_completed(queue, send_id, 30)
    harness.assert(completed ~= nil, subject_tag .. " missing action.completed")
    return subject
end

local primary_record = { account_id = primary.account_id, email = "primary@example.com" }
local secondary_record = { account_id = secondary.account_id, email = "secondary@example.com" }

local primary_subject = send_from(primary_record, "primary")
local secondary_subject = send_from(secondary_record, "secondary")

local submissions = wait_for_submissions(log_url, 2, 30)
harness.assert(submissions ~= nil, "expected exactly two SMTP submissions")

local primary_submission = submission_by_subject(submissions, primary_subject)
harness.assert(primary_submission ~= nil, "missing primary submission")
harness.assert_eq(primary_submission.account_id, "account-primary", "primary submission account_id")
harness.assert_eq(
    primary_submission.auth_mechanism,
    "PLAIN",
    "primary submission auth_mechanism"
)
harness.assert_eq(
    primary_submission.from,
    "<primary@example.com>",
    "primary submission from"
)
harness.assert_eq(
    primary_submission.recipients[1],
    "<primary-recipient@example.test>",
    "primary submission recipient"
)

local secondary_submission = submission_by_subject(submissions, secondary_subject)
harness.assert(secondary_submission ~= nil, "missing secondary submission")
harness.assert_eq(
    secondary_submission.account_id,
    "account-secondary",
    "secondary submission account_id"
)
harness.assert_eq(
    secondary_submission.auth_mechanism,
    "PLAIN",
    "secondary submission auth_mechanism"
)
harness.assert_eq(
    secondary_submission.from,
    "<secondary@example.com>",
    "secondary submission from"
)
harness.assert_eq(
    secondary_submission.recipients[1],
    "<secondary-recipient@example.test>",
    "secondary submission recipient"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
