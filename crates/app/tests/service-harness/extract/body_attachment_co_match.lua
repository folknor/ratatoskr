-- description: search attribution reports body and attachment co-matches
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

local function search(client, account_id, query)
    local result, result_err = client:request("TestSearchIndex", {
        account_id = account_id,
        query = query,
        limit = 10,
    })
    harness.assert(result_err == nil, "TestSearchIndex failed")
    return result
end

local function wait_for_result(client, account_id, query, message_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local result = search(client, account_id, query)
        for _, row in ipairs(result.results) do
            if row.message_id == message_id then
                return row
            end
        end

        harness.sleep(250)
    end
    return nil
end

local function attachment_match(matches, attachment_id)
    for _, matched in ipairs(matches) do
        if matched.kind == "attachment" and matched.attachment_id == attachment_id then
            return matched
        end
    end
    return nil
end

local dir = harness.data_dir("extract_body_attachment_co_match")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty kick finish before injecting cached rows.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-comatch@example.test",
    display_name = "Extract Co-match",
    account_name = "Extract Co-match",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract body and attachment co-match",
    label_ids = { "INBOX" },
    is_read = true,
    body_text = "contract terms live in the message body",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-comatch-attachment",
    filename = "contract.txt",
    mime_type = "text/plain",
    content = "contract appendix text from the attachment",
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")

request_backfill(client)

local indexed = wait_for_indexed(
    client,
    account.account_id,
    attachment.attachment_id,
    30
)
harness.assert(indexed ~= nil, "attachment was not indexed")

local result = wait_for_result(
    client,
    account.account_id,
    "contract",
    thread.message_id,
    30
)
harness.assert(result ~= nil, "search result was not indexed")
harness.assert_eq(result.match_kind.kind, "body", "primary match kind")

local matched_attachment = attachment_match(result.also_matched, attachment.attachment_id)
harness.assert(matched_attachment ~= nil, "attachment was not reported as also matched")
harness.assert_eq(matched_attachment.filename, "contract.txt", "attachment match filename")
harness.assert_eq(matched_attachment.mime, "text/plain", "attachment match mime")

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 1, "indexed_total did not advance")
harness.assert_eq(status.failed_total, 0, "failed total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
