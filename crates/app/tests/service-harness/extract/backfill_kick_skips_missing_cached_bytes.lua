-- description: extract backfill records bytes_gone when cached bytes vanish
-- ceiling: 90s

local function attachment_by_id(state, attachment_id)
    for _, attachment in ipairs(state.attachments) do
        if attachment.id == attachment_id then
            return attachment
        end
    end
    return nil
end

local function request_backfill(client)
    local ok, notify_err = client:notify("extract.backfill_kick")
    harness.assert(ok, "extract.backfill_kick send failed")
    harness.assert(notify_err == nil, "extract.backfill_kick returned error")
end

local function query_attachment(client, account_id, attachment_id)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
        attachment_limit = 10,
    })
    harness.assert(state_err == nil, "TestQueryDbState failed")
    return attachment_by_id(state, attachment_id)
end

local function wait_for_extraction_status(client, account_id, attachment_id, status, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local attachment = query_attachment(client, account_id, attachment_id)
        if attachment ~= nil and attachment.extraction_status == status then
            return attachment
        end

        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("extract_backfill_missing_cached_bytes")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty kick finish before injecting the cached row.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-bytes-gone@example.test",
    display_name = "Extract Bytes Gone",
    account_name = "Extract Bytes Gone",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract bytes gone",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-bytes-gone-text",
    filename = "bytes-gone.txt",
    mime_type = "text/plain",
    content = "phase seven bytes gone text",
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")

local removed, remove_err = client:request("TestRemoveCachedAttachmentBytes", {
    relative_path = attachment.relative_path,
})
harness.assert(remove_err == nil, "TestRemoveCachedAttachmentBytes failed")
harness.assert(removed.removed, "cached bytes were not removed")

local before = query_attachment(
    client,
    account.account_id,
    attachment.attachment_id
)
harness.assert(before ~= nil, "seeded attachment row missing")
harness.assert_eq(before.local_path, attachment.relative_path, "cache metadata path")
harness.assert(before.cached_at ~= nil, "cache metadata timestamp")
harness.assert(before.text_indexed_at == nil, "initial text indexed marker")
harness.assert(before.extraction_status == nil, "initial extraction status")

request_backfill(client)
local skipped = wait_for_extraction_status(
    client,
    account.account_id,
    attachment.attachment_id,
    "skipped:bytes_gone",
    30
)
harness.assert(skipped ~= nil, "missing bytes were not recorded as bytes_gone")
harness.assert(skipped.text_indexed_at == nil, "bytes_gone text indexed marker")
harness.assert(skipped.extracted_text == nil, "bytes_gone extracted text")

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert_eq(status.indexed_total, 0, "indexed total")
harness.assert_eq(status.skipped_total, 1, "skipped total")
harness.assert_eq(status.failed_total, 0, "failed total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
