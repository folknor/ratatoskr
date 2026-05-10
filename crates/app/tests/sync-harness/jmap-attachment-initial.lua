-- description: JMAP initial sync imports attachment metadata from raw bytes
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
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

local dir = harness.data_dir("sync_jmap_attachment_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-attachment@example.test",
    display_name = "Sync JMAP Attachment",
    account_name = "Sync JMAP Attachment",
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
local email_get_requests = harness.request_count(requests, "jmap", "Email/get")
harness.assert(email_get_requests >= 1, "JMAP attachment sync did not call Email/get")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    attachment_count = state.attachment_count,
    provider_requests = #requests,
    jmap_email_get_requests = email_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
