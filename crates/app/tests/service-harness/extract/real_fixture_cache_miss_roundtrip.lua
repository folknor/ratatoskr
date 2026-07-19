-- description: real extraction fixtures fetch from provider cache miss and reach search
-- fixture: jmap-extract.toml
-- protocol: jmap
-- ceiling: 120s

-- B9: this roundtrip proves the resident bifrost engine byte source
-- (crates/service/src/bifrost/attachment.rs, open_blob) hydrates real
-- attachment bytes on a genuine cache-miss, and that the fetched bytes
-- reach the extraction + search pipeline.
--
-- The corpus (real PDF/DOCX/XLSX/PPTX plus a zipbomb-shaped .docx) is
-- served over JMAP by saehrimnir from the jmap-extract.toml fixture
-- (blobs/extract/*). The pre-B9 version of this gate seeded bytes into
-- the legacy HarnessOfflineProvider registry and relied on a
-- per-provider cache-miss fetch that attached a `harness-offline`
-- provider; the B9 engine byte source cannot attach that provider, so
-- the fetch is now driven through a real JMAP mock the resident engine
-- CAN attach - the same path the jmap/gmail/graph/imap-attachment-*
-- gates exercise.
--
-- Cache-miss is forced structurally: `cache_attachments_enabled = 0`
-- makes initial sync land the attachment rows NULL-hashed (no prefetch
-- bytes), so the first AttachmentFetch per row is the first byte fetch
-- and drives open_blob. A second AttachmentFetch on an already-cached
-- row takes handle_fetch's cache-hit branch (content_hash set +
-- blob_present) and returns the same hash without a provider fetch.

local FIXTURES = {
    {
        filename = "known-content.pdf",
        token = "pdfrealfixture",
        status = "indexed",
    },
    {
        filename = "known-content.docx",
        token = "docxrealfixture",
        status = "indexed",
    },
    {
        filename = "known-content.xlsx",
        token = "xlsxrealfixture",
        status = "indexed",
    },
    {
        filename = "known-content.pptx",
        token = "pptxrealfixture",
        status = "indexed",
    },
    {
        filename = "zipbomb-shaped.docx",
        token = nil,
        status = "skipped:zipbomb",
    },
}

local function attachment_by_filename(state, filename)
    for _, attachment in ipairs(state.attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local function query_state(client, account_id)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
        attachment_limit = 20,
    })
    harness.assert(state_err == nil, "TestQueryDbState failed")
    return state
end

local function wait_for_attachment_status(client, account_id, filename, status, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local attachment = attachment_by_filename(query_state(client, account_id), filename)
        if attachment ~= nil and attachment.extraction_status == status then
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

local function wait_for_attachment_match(client, account_id, message_id, attachment_id, query, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local results = search(client, account_id, query)
        for _, result in ipairs(results.results) do
            if result.message_id == message_id
                and result.match_kind ~= nil
                and result.match_kind.kind == "attachment"
                and result.match_kind.attachment_id == attachment_id
            then
                return result
            end
        end
        harness.sleep(250)
    end
    return nil
end

-- Cache-miss fetch of one row: assert it is NOT yet cached, drive the
-- engine byte source through AttachmentFetch, assert the bytes landed.
local function fetch_cache_miss(client, account_id, row)
    harness.assert(row.content_hash == nil, row.filename .. " unexpectedly cached before fetch")
    local fetched, fetch_err = client:request("AttachmentFetch", {
        account_id = account_id,
        message_id = row.message_id,
        attachment_id = row.id,
    })
    harness.assert(fetch_err == nil, row.filename .. " attachment.fetch failed: " .. tostring(fetch_err))
    harness.assert(fetched.content_hash ~= nil, row.filename .. " fetch missing content hash")
    harness.assert(fetched.relative_path ~= nil, row.filename .. " fetch missing relative path")
    return fetched
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("extract_real_fixture_cache_miss")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-real-fixture@example.test",
    display_name = "Extract Real Fixture",
    account_name = "Extract Real Fixture",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- Disable the per-account offline cache so initial sync does NOT
-- prefetch bytes: attachment rows land with content_hash IS NULL, and
-- the first AttachmentFetch per row is a genuine cache-miss that drives
-- the resident engine byte source.
local _, disable_err = client:request("AccountUpdate", {
    id = account.account_id,
    cache_attachments_enabled = false,
})
harness.assert(disable_err == nil, "AccountUpdate (disable cache) failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- All five attachment rows hydrated from sync, NULL-hashed.
local state = query_state(client, account.account_id)
harness.assert_eq(state.message_count, 1, "message count")
harness.assert_eq(state.attachment_count, 5, "attachment count")

local rows = {}
for _, fixture in ipairs(FIXTURES) do
    local row = attachment_by_filename(state, fixture.filename)
    harness.assert(row ~= nil, fixture.filename .. " row missing after sync")
    rows[fixture.filename] = row
end

-- Cache-miss fetch every attachment through the engine byte source.
local message_id = nil
for _, fixture in ipairs(FIXTURES) do
    local row = rows[fixture.filename]
    local fetched = fetch_cache_miss(client, account.account_id, row)
    rows[fixture.filename] = row
    message_id = row.message_id
    fixture.attachment_id = row.id
    fixture.content_hash = fetched.content_hash
end
harness.assert(message_id ~= nil, "message id missing")

-- Second read of an already-cached row is a cache hit: same content
-- hash, no provider fetch (handle_fetch's cache-hit branch skips the
-- byte source when content_hash is set and the blob is present).
local pdf_row = rows["known-content.pdf"]
local hit, hit_err = client:request("AttachmentFetch", {
    account_id = account.account_id,
    message_id = pdf_row.message_id,
    attachment_id = pdf_row.id,
})
harness.assert(hit_err == nil, "cache-hit re-fetch failed: " .. tostring(hit_err))
harness.assert_eq(hit.content_hash, FIXTURES[1].content_hash, "cache-hit content hash mismatch")
harness.assert(hit.relative_path ~= nil, "cache-hit missing relative path")

-- Extraction indexes each real fixture (zipbomb-shaped is skipped).
for _, fixture in ipairs(FIXTURES) do
    local indexed =
        wait_for_attachment_status(client, account.account_id, fixture.filename, fixture.status, 30)
    harness.assert(
        indexed ~= nil,
        fixture.filename .. " did not reach status " .. fixture.status
    )
    if fixture.token ~= nil then
        harness.assert(
            string.find(indexed.extracted_text or "", fixture.token, 1, true) ~= nil,
            fixture.filename .. " extracted text missing fixture token " .. fixture.token
        )
    end
end

-- Each indexed fixture is reachable through attachment search.
for _, fixture in ipairs(FIXTURES) do
    if fixture.token ~= nil then
        local match = wait_for_attachment_match(
            client,
            account.account_id,
            message_id,
            fixture.attachment_id,
            fixture.token,
            30
        )
        harness.assert(match ~= nil, fixture.filename .. " search match missing")
        harness.assert_eq(match.match_kind.filename, fixture.filename, fixture.filename .. " match filename")
    end
end

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 4, "indexed_total did not include all indexed fixtures")
harness.assert_eq(status.failed_total, 0, "failed total")

harness.write_summary({
    correct = 1,
    attachment_count = state.attachment_count,
    indexed_total = status.indexed_total,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
