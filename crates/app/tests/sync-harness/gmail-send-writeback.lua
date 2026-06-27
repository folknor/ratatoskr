-- description: Gmail client send dispatches via messages.send and the sent message round-trips into SENT
-- expected: pass
-- fixture: send-small.toml
-- protocol: gmail
-- ceiling: 120s
--
-- Per-provider send-writeback gate (B5-GATES). Drives the real send action
-- (ActionSend -> resident SyncEngine engine.send_message -> Gmail messages.send
-- against saehrimnir), then resyncs and asserts the sent message round-trips
-- into the server's SENT label. See jmap-send-writeback.lua for the two-layered
-- verification rationale (remote-dispatch summary + server round-trip).

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

-- Gmail is OAuth-only: mint a bearer for the fixture's primary account
-- (account-1) off the mock OAuth provider before seeding.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local token_response = harness.http_json({
    method = "POST",
    url = token_url,
    body = {
        grant_type = "authorization_code",
        account_id = "account-1",
        code = "harness-gmail-send-account-1",
        client_id = "ratatoskr-gmail-harness",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(token_response.access_token ~= nil, "/oauth/token did not return access_token")

local dir = harness.data_dir("sync_gmail_send_writeback")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Sync Gmail Send",
    account_name = "Sync Gmail Send",
    provider = "gmail_api",
    access_token = token_response.access_token,
    refresh_token = "gmail-send-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local send_id = harness.uuid()
local subject = "gmail send roundtrip " .. send_id
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
        body_html = "<p>gmail send roundtrip</p>",
        body_text = "gmail send roundtrip",
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
