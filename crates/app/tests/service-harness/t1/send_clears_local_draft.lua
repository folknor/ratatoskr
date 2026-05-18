-- description: successful action.send removes the local_drafts row
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
        if submissions ~= nil and #submissions == count then
            return submissions
        end
        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("t1_send_clears_local_draft")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local account, account_err = client:request("TestSeedAccount", {
    email = "draft-cleanup@example.test",
    display_name = "Draft Cleanup",
    account_name = "Draft Cleanup",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local queue = client:notifications()
local send_id = harness.uuid()
local draft_id = "draft-" .. send_id

local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = account.account_id,
    message = {
        draft_id = draft_id,
        from = "draft-cleanup@example.test",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = "send clears local draft",
        body_html = "<p>regression test</p>",
        body_text = "regression test",
    },
    attachments = {},
})
harness.assert(ack_err == nil, "action.send failed")
harness.assert(ack.journaled, "send was not journaled")

local completed = wait_for_action_completed(queue, send_id, 30)
harness.assert(completed ~= nil, "missing action.completed")
harness.assert_eq(completed.summary_total, 1, "send completion total")

local submissions = wait_for_submissions(log_url, 1, 30)
harness.assert(submissions ~= nil, "missing SMTP submission")

-- After a successful send, the local_drafts row must be gone. Pre-fix
-- behaviour: the row lingered with sync_status='sent', surfacing as a
-- phantom entry in the Drafts pane forever.
local snapshot, snapshot_err = client:request("TestQueryDbState", {})
harness.assert(snapshot_err == nil, "TestQueryDbState failed")
harness.assert_eq(
    snapshot.local_draft_count,
    0,
    "local_drafts row should be deleted on send success"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
