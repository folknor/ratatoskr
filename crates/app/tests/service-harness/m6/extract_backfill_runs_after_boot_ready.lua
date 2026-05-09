-- description: post-ready extract startup backfills cached unindexed attachments
-- ceiling: 90s

local function attachment_by_id(state, attachment_id)
    for _, attachment in ipairs(state.attachments) do
        if attachment.id == attachment_id then
            return attachment
        end
    end
    return nil
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

local function wait_for_indexed(client, account_id, attachment_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local attachment = query_attachment(client, account_id, attachment_id)
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

local dir = harness.data_dir("m6_extract_backfill_after_boot_ready")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "initial spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "initial boot.ready failed")
harness.assert(ready.ready, "initial boot.ready returned ready=false")

-- Let the first post-ready empty backfill pass finish before seeding
-- the attachment that the second boot must discover.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-boot-backfill@example.test",
    display_name = "Extract Boot Backfill",
    account_name = "Extract Boot Backfill",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract boot backfill",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local phrase = "boot ready backfill indexed text"
local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-boot-backfill-text",
    filename = "boot-backfill.txt",
    mime_type = "text/plain",
    content = phrase,
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")

local before = query_attachment(client, account.account_id, attachment.attachment_id)
harness.assert(before ~= nil, "seeded attachment row missing")
harness.assert(before.extraction_status == nil, "initial extraction status")
harness.assert(before.text_indexed_at == nil, "initial text indexed marker")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "initial shutdown failed")
harness.assert(shutdown_err == nil, "initial shutdown returned error")
client:drop()

local second, second_err = harness.spawn(dir)
harness.assert(second_err == nil, "second spawn failed")

local second_ready, second_ready_err = second:request("BootReady")
harness.assert(second_ready_err == nil, "second boot.ready failed")
harness.assert(second_ready.ready, "second boot.ready returned ready=false")

local indexed = wait_for_indexed(second, account.account_id, attachment.attachment_id, 30)
harness.assert(indexed ~= nil, "post-ready backfill did not index cached attachment")
harness.assert_eq(indexed.extracted_text, phrase, "extracted text")

local status, status_err = second:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 1, "indexed_total did not advance")
harness.assert_eq(status.failed_total, 0, "failed total")

local second_ok, second_shutdown_err = second:shutdown()
harness.assert(second_ok, "second shutdown failed")
harness.assert(second_shutdown_err == nil, "second shutdown returned error")
