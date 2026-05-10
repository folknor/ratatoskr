-- description: JMAP initial sync tolerates malformed raw body bytes
-- expected: pass
-- fixture: jmap-malformed-mime.lua
-- protocol: jmap
-- ceiling: 120s

local function message_by_id(messages, id)
    for _, message in ipairs(messages) do
        if message.id == id then
            return message
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_malformed_mime_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-malformed-mime@example.test",
    display_name = "Sync JMAP Malformed MIME",
    account_name = "Sync JMAP Malformed MIME",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 1, "message count")
local message = message_by_id(state.messages, "malformed-001")
harness.assert(message ~= nil, "missing malformed message")
harness.assert_eq(message.subject, "Malformed multipart boundary", "malformed subject")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local email_get_requests = harness.request_count(requests, "jmap", "Email/get")
harness.assert(email_get_requests >= 1, "JMAP malformed sync did not call Email/get")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    provider_requests = #requests,
    jmap_email_get_requests = email_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
