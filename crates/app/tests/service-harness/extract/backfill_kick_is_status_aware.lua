-- description: repeat extract backfill does not re-index resolved cached text
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

local function assert_status_stable(client, baseline, seconds)
    local deadline = harness.now_ms() + seconds * 1000
    while harness.now_ms() < deadline do
        local status, status_err = client:request("ExtractStatus")
        harness.assert(status_err == nil, "extract.status failed")
        harness.assert_eq(status.queue_depth, 0, "queue depth")
        harness.assert_eq(status.indexed_total, baseline.indexed_total, "indexed total")
        harness.assert_eq(status.skipped_total, baseline.skipped_total, "skipped total")
        harness.assert_eq(status.failed_total, baseline.failed_total, "failed total")
        harness.sleep(250)
    end
end

local dir = harness.data_dir("extract_backfill_status_aware")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty kick finish before injecting the cached row.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-idempotent@example.test",
    display_name = "Extract Idempotent",
    account_name = "Extract Idempotent",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract idempotent",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local phrase = "phase seven idempotent backfill text"
local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-idempotent-text",
    filename = "idempotent.txt",
    mime_type = "text/plain",
    content = phrase,
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")

request_backfill(client)
local indexed = wait_for_indexed(client, account.account_id, attachment.attachment_id, 30)
harness.assert(indexed ~= nil, "cached attachment was not indexed")
harness.assert_eq(indexed.extracted_text, phrase, "extracted text")

local baseline, baseline_err = client:request("ExtractStatus")
harness.assert(baseline_err == nil, "baseline extract.status failed")
harness.assert_eq(baseline.queue_depth, 0, "baseline queue depth")
harness.assert_eq(baseline.indexed_total, 1, "baseline indexed total")
harness.assert_eq(baseline.failed_total, 0, "baseline failed total")

request_backfill(client)
assert_status_stable(client, baseline, 2)

local after = query_attachment(client, account.account_id, attachment.attachment_id)
harness.assert(after ~= nil, "attachment row missing after repeat kick")
harness.assert_eq(after.text_indexed_at, indexed.text_indexed_at, "text indexed marker")
harness.assert_eq(after.extraction_status, "indexed", "extraction status")
harness.assert_eq(after.extracted_text, phrase, "extracted text after repeat kick")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
