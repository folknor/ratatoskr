-- description: compose send submits a 50 MB staged attachment through SMTP
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 240s

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
        if #submissions == count then
            return submissions
        end
        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("t1_compose_send_50mb")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local account, account_err = client:request("TestSeedAccount", {
    email = "compose-large@example.test",
    display_name = "Compose Large",
    account_name = "Compose Large",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local queue = client:notifications()
local send_id = harness.uuid()
local size = 50 * 1024 * 1024
local staged = harness.stage_attachment(dir, send_id, 0, {
    byte = "x",
    size = size,
})

local subject = "compose send 50mb attachment"
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    message = {
        draft_id = "draft-compose-large",
        from = "compose-large@example.test",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = subject,
        body_html = "<p>Large attachment</p>",
        body_text = "Large attachment",
    },
    attachments = {
        {
            source = staged.source,
            size = staged.size,
            mime = "text/plain",
            filename = "large-50mb.txt",
        },
    },
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "send was not journaled")
harness.assert_eq(ack.send_id, send_id, "send ack id")

local completed = wait_for_action_completed(queue, send_id, 120)
harness.assert(completed ~= nil, "missing action.completed")
harness.assert_eq(completed.summary_total, 1, "send completion total")

local submissions = wait_for_submissions(log_url, 1, 30)
harness.assert(submissions ~= nil, "missing SMTP submission")
local submission = submissions[1]
harness.assert_eq(submission.from, "<compose-large@example.test>", "SMTP from")
harness.assert_eq(submission.recipients[1], "<recipient@example.test>", "SMTP recipient")
harness.assert(submission.parsed ~= nil, "submission did not parse")
harness.assert_eq(submission.parsed.subject, subject, "parsed subject")
harness.assert_eq(#submission.parsed.attachments, 1, "attachment count")
local attachment = submission.parsed.attachments[1]
harness.assert_eq(attachment.filename, "large-50mb.txt", "attachment filename")
harness.assert_eq(attachment.size, size, "attachment size")
harness.assert(
    string.find(attachment.content_type, "text/plain", 1, true) ~= nil,
    "attachment content type"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
