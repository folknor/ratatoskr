-- description: extract backfill indexes a cached text attachment
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

local function wait_for_indexed(client, account_id, attachment_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
            message_limit = 10,
            attachment_limit = 10,
        })
        harness.assert(state_err == nil, "TestQueryDbState while waiting failed")

        local attachment = attachment_by_id(state, attachment_id)
        if attachment ~= nil
            and attachment.extraction_status == "indexed"
            and attachment.text_indexed_at ~= nil
        then
            return attachment
        end

        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("extract_backfill_cached_text")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Give the post-ready extract startup task time to run its initial
-- empty kick before this script injects the cached attachment.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-backfill@example.test",
    display_name = "Extract Backfill",
    account_name = "Extract Backfill",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract backfill",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local phrase = "phase seven backfill unique text"
local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-backfill-text",
    filename = "backfill.txt",
    mime_type = "text/plain",
    content = phrase,
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")
harness.assert_eq(attachment.size_bytes, string.len(phrase), "cached size")
harness.assert_eq(
    attachment.relative_path,
    "attachment_cache/" .. attachment.content_hash,
    "cache path"
)

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
    attachment_limit = 10,
})
harness.assert(before_err == nil, "initial TestQueryDbState failed")
local before_attachment = attachment_by_id(before, attachment.attachment_id)
harness.assert(before_attachment ~= nil, "seeded attachment row missing")
harness.assert(before_attachment.extraction_status == nil, "initial extraction status")
harness.assert(before_attachment.text_indexed_at == nil, "initial text indexed marker")

request_backfill(client)
local indexed = wait_for_indexed(client, account.account_id, attachment.attachment_id, 30)
harness.assert(indexed ~= nil, "cached attachment was not indexed")
harness.assert_eq(indexed.content_hash, attachment.content_hash, "indexed content hash")
harness.assert_eq(indexed.extracted_text, phrase, "extracted text")

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert(status.indexed_total >= 1, "indexed_total did not advance")
harness.assert_eq(status.failed_total, 0, "failed_total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
