-- description: reading an MDN-requesting message sends a read receipt that round-trips into SENT
-- expected: pass
-- fixture: mdn-small.toml
-- protocol: gmail
-- ceiling: 120s
--
-- MDN (read-receipt) round-trip gate for an HTTP-API provider (B5-GATES).
-- Gmail hydrates raw RFC822 via messages.get format=raw, so the consumer sees
-- the fixture message's Disposition-Notification-To and sets mdn_requested = 1.
-- With an "always" read-receipt policy, marking the thread read dispatches the
-- RFC 8098 MDN via engine.send_raw_message -> Gmail messages.send. The gate
-- resyncs and asserts the MDN round-tripped into SENT addressed to the original
-- sender (the HTTP-API analogue of imap-mdn.lua's SMTP-log check).

local function has_label(labels, expected)
    for _, label in ipairs(labels) do
        if label == expected then
            return true
        end
    end
    return false
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

-- Gmail is OAuth-only: mint a bearer for the fixture's primary account.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local token_response = harness.http_json({
    method = "POST",
    url = token_url,
    body = {
        grant_type = "authorization_code",
        account_id = "account-1",
        code = "harness-gmail-mdn-account-1",
        client_id = "ratatoskr-gmail-harness",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(token_response.access_token ~= nil, "/oauth/token did not return access_token")

local dir = harness.data_dir("sync_gmail_mdn")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Sync Gmail MDN",
    account_name = "Sync Gmail MDN",
    provider = "gmail_api",
    access_token = token_response.access_token,
    refresh_token = "gmail-mdn-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
    read_receipt_policy = "always",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(before_err == nil, "TestQueryDbState (before) failed")
local message = message_by_subject(before.messages, "Please confirm receipt")
harness.assert(message ~= nil, "MDN-requesting message did not sync")

-- Mark read; the read-receipt follow-up dispatches the MDN.
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

-- Round-trip: resync and find the MDN filed under SENT, addressed to the sender.
local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-read resync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-read resync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(after_err == nil, "TestQueryDbState (after) failed")

local mdn = nil
for _, candidate in ipairs(after.messages) do
    local to = candidate.to_addresses
    if to ~= nil and string.find(to, "sender@example.com", 1, true) ~= nil then
        local thread = client:request("TestThreadRead", {
            account_id = account.account_id,
            thread_id = candidate.thread_id,
        })
        if thread ~= nil and thread.label_ids ~= nil and has_label(thread.label_ids, "SENT") then
            mdn = candidate
            break
        end
    end
end
harness.assert(mdn ~= nil, "MDN did not round-trip into SENT addressed to the sender")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
