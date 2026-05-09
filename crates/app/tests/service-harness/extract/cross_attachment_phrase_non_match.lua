-- description: extracted attachment phrase search does not cross attachment boundaries
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

local function wait_for_hit(client, account_id, query, message_id, timeout)
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

local dir = harness.data_dir("extract_cross_attachment_phrase_non_match")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Let the post-ready empty kick finish before injecting cached rows.
harness.sleep(1000)

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-boundary@example.test",
    display_name = "Extract Boundary",
    account_name = "Extract Boundary",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract phrase boundary",
    label_ids = { "INBOX" },
    is_read = true,
    body_text = "message body intentionally avoids the attachment phrase tokens",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local first_attachment, first_attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-boundary-one",
    filename = "boundary-one.txt",
    mime_type = "text/plain",
    content = "the quick brown",
})
harness.assert(first_attachment_err == nil, "first TestSeedCachedAttachment failed")

local second_attachment, second_attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = account.account_id,
    message_id = thread.message_id,
    attachment_id = "extract-boundary-two",
    filename = "boundary-two.txt",
    mime_type = "text/plain",
    content = "fox jumps onward",
})
harness.assert(second_attachment_err == nil, "second TestSeedCachedAttachment failed")

request_backfill(client)

local first_indexed = wait_for_indexed(
    client,
    account.account_id,
    first_attachment.attachment_id,
    30
)
harness.assert(first_indexed ~= nil, "first attachment was not indexed")

local second_indexed = wait_for_indexed(
    client,
    account.account_id,
    second_attachment.attachment_id,
    30
)
harness.assert(second_indexed ~= nil, "second attachment was not indexed")

local first_hit = wait_for_hit(
    client,
    account.account_id,
    "\"the quick\"",
    thread.message_id,
    30
)
harness.assert(first_hit ~= nil, "within first attachment phrase did not hit")

local second_hit = wait_for_hit(
    client,
    account.account_id,
    "\"fox jumps\"",
    thread.message_id,
    30
)
harness.assert(second_hit ~= nil, "within second attachment phrase did not hit")

local cross = search(client, account.account_id, "\"brown fox\"")
harness.assert_eq(cross.total, 0, "cross-attachment phrase result count")

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 2, "indexed_total did not advance for both attachments")
harness.assert_eq(status.failed_total, 0, "failed total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
