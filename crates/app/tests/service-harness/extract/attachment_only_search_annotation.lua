-- description: attachment-only search reports attachment match attribution
-- ceiling: 90s

local unique = "saffronclause"
local attachment_text = "The archived attachment includes saffronclause evidence for search attribution."

local function request_backfill(client)
    local ok, notify_err = client:notify("extract.backfill_kick")
    harness.assert(ok, "extract.backfill_kick send failed")
    harness.assert(notify_err == nil, "extract.backfill_kick returned error")
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

local function wait_for_attachment_match(client, account_id, message_id, attachment_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000

    while harness.now_ms() < deadline do
        local results = search(client, account_id, unique)

        for _, result in ipairs(results.results) do
            if result.message_id == message_id then
                harness.assert(result.match_kind ~= nil, "search result missing match_kind")
                if result.match_kind.kind == "attachment" and result.match_kind.attachment_id == attachment_id then
                    return result
                end
            end
        end

        harness.sleep(250)
    end

    return nil
end

local dir = harness.data_dir("extract_attachment_only_search_annotation")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty backfill pass finish before we seed the attachment
-- that this script explicitly kicks below.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-attachment-only@example.test",
    display_name = "Extract Attachment Only",
    account_name = "Extract Attachment Only",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract attachment-only attribution",
    label_ids = { "INBOX" },
    is_read = true,
    body_text = "Plain body text without the special search token.",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local attachment, attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-attachment-only",
    filename = "evidence.txt",
    mime_type = "text/plain",
    content = attachment_text,
})
harness.assert(attachment_err == nil, "TestSeedCachedAttachment failed")

request_backfill(client)

local result = wait_for_attachment_match(
    client,
    account.account_id,
    thread.message_id,
    attachment.attachment_id,
    30
)
harness.assert(result ~= nil, "search result was not indexed with attachment attribution")
harness.assert(#result.also_matched == 0, "attachment-only search should not report secondary matches")
harness.assert_eq(result.match_kind.kind, "attachment", "primary match kind")
harness.assert_eq(result.match_kind.attachment_id, attachment.attachment_id, "primary attachment id")
harness.assert_eq(result.match_kind.filename, "evidence.txt", "primary attachment filename")
harness.assert_eq(result.match_kind.mime, "text/plain", "primary attachment mime")
harness.assert(
    string.find(result.match_kind.snippet, unique, 1, true) ~= nil,
    "primary attachment snippet missing search term"
)

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 1, "indexed_total did not advance")
harness.assert_eq(status.failed_total, 0, "failed total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
