-- description: extract backfill marks a new attachment sharing resolved content
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

local function query_state(client, account_id)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
        attachment_limit = 10,
    })
    harness.assert(state_err == nil, "TestQueryDbState failed")
    return state
end

local function query_attachment(client, account_id, attachment_id)
    return attachment_by_id(query_state(client, account_id), attachment_id)
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

local dir = harness.data_dir("extract_backfill_resolved_hash_reference")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty kick finish before injecting cached rows.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-dedupe@example.test",
    display_name = "Extract Dedupe",
    account_name = "Extract Dedupe",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local first_thread, first_thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract dedupe one",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(first_thread_err == nil, "first TestSeedThread failed")

local phrase = "phase seven dedupe shared attachment text"
local first_attachment, first_attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = first_thread.message_id,
    attachment_id = "extract-dedupe-one",
    filename = "dedupe-one.txt",
    mime_type = "text/plain",
    content = phrase,
})
harness.assert(first_attachment_err == nil, "first TestSeedCachedAttachment failed")

request_backfill(client)
local first_indexed = wait_for_indexed(
    client,
    account.account_id,
    first_attachment.attachment_id,
    30
)
harness.assert(first_indexed ~= nil, "first attachment was not indexed")
harness.assert_eq(first_indexed.extracted_text, phrase, "first extracted text")

local baseline, baseline_err = client:request("ExtractStatus")
harness.assert(baseline_err == nil, "baseline extract.status failed")
harness.assert_eq(baseline.queue_depth, 0, "baseline queue depth")
harness.assert_eq(baseline.indexed_total, 1, "baseline indexed total")
harness.assert_eq(baseline.failed_total, 0, "baseline failed total")

local second_thread, second_thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract dedupe two",
    label_ids = { "INBOX" },
    is_read = true,
})
harness.assert(second_thread_err == nil, "second TestSeedThread failed")

local second_attachment, second_attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = second_thread.message_id,
    attachment_id = "extract-dedupe-two",
    filename = "dedupe-two.txt",
    mime_type = "text/plain",
    content = phrase,
})
harness.assert(second_attachment_err == nil, "second TestSeedCachedAttachment failed")
harness.assert_eq(
    second_attachment.content_hash,
    first_attachment.content_hash,
    "shared content hash"
)

local before_second = query_attachment(
    client,
    account.account_id,
    second_attachment.attachment_id
)
harness.assert(before_second ~= nil, "second attachment row missing")
harness.assert_eq(before_second.extraction_status, "indexed", "shared extraction status")
harness.assert_eq(before_second.extracted_text, phrase, "shared extracted text")
harness.assert(before_second.text_indexed_at == nil, "second text indexed marker")

request_backfill(client)
local second_indexed = wait_for_indexed(
    client,
    account.account_id,
    second_attachment.attachment_id,
    30
)
harness.assert(second_indexed ~= nil, "second attachment was not marked indexed")
harness.assert_eq(second_indexed.extracted_text, phrase, "second extracted text")

local after_status, after_status_err = client:request("ExtractStatus")
harness.assert(after_status_err == nil, "after extract.status failed")
harness.assert_eq(after_status.queue_depth, 0, "after queue depth")
harness.assert_eq(after_status.indexed_total, baseline.indexed_total, "after indexed total")
harness.assert_eq(after_status.skipped_total, baseline.skipped_total, "after skipped total")
harness.assert_eq(after_status.failed_total, baseline.failed_total, "after failed total")

local after_first = query_attachment(
    client,
    account.account_id,
    first_attachment.attachment_id
)
harness.assert(after_first ~= nil, "first attachment row missing after second kick")
harness.assert_eq(
    after_first.text_indexed_at,
    first_indexed.text_indexed_at,
    "first text indexed marker"
)
harness.assert_eq(after_first.extracted_text, phrase, "first extracted text after second kick")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
