-- description: compose send preserves staged attachment metadata on the SMTP wire
-- fixture: jmap-small.toml
-- protocol: jmap
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
        if #submissions == count then
            return submissions
        end
        harness.sleep(250)
    end
    return nil
end

local function attachment_by_name(parsed, name)
    for _, attachment in ipairs(parsed.attachments) do
        if attachment.filename == name then
            return attachment
        end
    end
    return nil
end

local dir = harness.data_dir("t1_send_wire_attachment_validation")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local account, account_err = client:request("TestSeedAccount", {
    email = "compose-validation@example.test",
    display_name = "Compose Validation",
    account_name = "Compose Validation",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local queue = client:notifications()
local send_id = harness.uuid()
local text_payload = "plain attachment body"
local json_payload = "{\"kind\":\"validation\",\"ok\":true}"
local text_staged = harness.stage_attachment(dir, send_id, 0, text_payload)
local json_staged = harness.stage_attachment(dir, send_id, 1, json_payload)

local subject = "compose send attachment validation"
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    message = {
        draft_id = "draft-compose-validation",
        from = "compose-validation@example.test",
        to = { "recipient@example.test" },
        cc = { "copy@example.test" },
        bcc = {},
        subject = subject,
        body_html = "<p>Attachment validation</p>",
        body_text = "Attachment validation",
    },
    attachments = {
        {
            source = text_staged.source,
            size = text_staged.size,
            mime = "text/plain",
            filename = "plain.txt",
        },
        {
            source = json_staged.source,
            size = json_staged.size,
            mime = "application/json",
            filename = "payload.json",
        },
    },
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "send was not journaled")

local completed = wait_for_action_completed(queue, send_id, 30)
harness.assert(completed ~= nil, "missing action.completed")

local submissions = wait_for_submissions(log_url, 1, 10)
harness.assert(submissions ~= nil, "missing SMTP submission")
local submission = submissions[1]
harness.assert_eq(submission.from, "<compose-validation@example.test>", "SMTP from")
harness.assert_eq(submission.recipients[1], "<recipient@example.test>", "SMTP to")
harness.assert_eq(submission.recipients[2], "<copy@example.test>", "SMTP cc")
harness.assert(submission.parsed ~= nil, "submission did not parse")
harness.assert_eq(submission.parsed.subject, subject, "parsed subject")
harness.assert_eq(#submission.parsed.attachments, 2, "attachment count")

local text_attachment = attachment_by_name(submission.parsed, "plain.txt")
harness.assert(text_attachment ~= nil, "missing plain.txt")
harness.assert_eq(text_attachment.size, string.len(text_payload), "plain size")
harness.assert(
    string.find(text_attachment.content_type, "text/plain", 1, true) ~= nil,
    "plain content type"
)

local json_attachment = attachment_by_name(submission.parsed, "payload.json")
harness.assert(json_attachment ~= nil, "missing payload.json")
harness.assert_eq(json_attachment.size, string.len(json_payload), "json size")
harness.assert(
    string.find(json_attachment.content_type, "application/json", 1, true) ~= nil,
    "json content type"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
