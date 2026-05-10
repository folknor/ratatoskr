-- description: Gmail initial sync imports attachment metadata from thread raw bytes
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: gmail
-- ceiling: 120s

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_gmail_attachment_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-attachment@example.test",
    display_name = "Sync Gmail Attachment",
    account_name = "Sync Gmail Attachment",
    provider = "gmail_api",
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
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 1, "message count")
harness.assert_eq(state.attachment_count, 1, "attachment count")

local attachment = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(attachment ~= nil, "missing sample.txt attachment")
harness.assert_eq(attachment.mime_type, "text/plain", "attachment mime type")
harness.assert((attachment.size or 0) > 0, "attachment size")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local thread_get_requests =
    harness.request_count_prefix(requests, "gmail", "GET /gmail/v1/users/me/threads/")
harness.assert(thread_get_requests >= 1, "Gmail attachment sync did not fetch a thread")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    attachment_count = state.attachment_count,
    provider_requests = #requests,
    gmail_thread_get_requests = thread_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
