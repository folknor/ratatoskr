-- description: real extraction fixtures fetch from provider cache miss and reach search
-- ceiling: 120s

local fixture_root = "crates/app/tests/service-harness/fixtures/extract"

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
        attachment_limit = 20,
    })
    harness.assert(state_err == nil, "TestQueryDbState failed")
    return attachment_by_id(state, attachment_id)
end

local function wait_for_attachment_status(client, account_id, attachment_id, status, timeout)
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

local function seed_remote_attachment(client, account_id, message_id, spec)
    local ack, err = client:request("TestSeedRemoteAttachment", {
        account_id = account_id,
        message_id = message_id,
        attachment_id = spec.attachment_id,
        filename = spec.filename,
        mime_type = spec.mime_type,
        content_base64 = harness.read_base64(fixture_root .. "/" .. spec.fixture),
    })
    harness.assert(err == nil, spec.filename .. " TestSeedRemoteAttachment failed")

    local before = query_attachment(client, account_id, ack.attachment_id)
    harness.assert(before ~= nil, spec.filename .. " row missing before fetch")
    harness.assert(before.content_hash == nil, spec.filename .. " unexpectedly cached before fetch")

    local fetched, fetch_err = client:request("AttachmentFetch", {
        account_id = account_id,
        message_id = message_id,
        attachment_id = ack.attachment_id,
    })
    harness.assert(fetch_err == nil, spec.filename .. " attachment.fetch failed")
    harness.assert(fetched.content_hash ~= nil, spec.filename .. " fetch missing content hash")
    harness.assert(fetched.relative_path ~= nil, spec.filename .. " fetch missing relative path")
    return ack
end

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
    provider = "harness-offline",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract real fixture corpus",
    label_ids = { "INBOX" },
    is_read = true,
    body_text = "Message body intentionally avoids the fixture search tokens.",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local pdf = seed_remote_attachment(client, account.account_id, thread.message_id, {
    attachment_id = "extract-real-pdf",
    filename = "known-content.pdf",
    mime_type = "application/pdf",
    fixture = "known-content.pdf",
})
local docx = seed_remote_attachment(client, account.account_id, thread.message_id, {
    attachment_id = "extract-real-docx",
    filename = "known-content.docx",
    mime_type = "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    fixture = "known-content.docx",
})
local xlsx = seed_remote_attachment(client, account.account_id, thread.message_id, {
    attachment_id = "extract-real-xlsx",
    filename = "known-content.xlsx",
    mime_type = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    fixture = "known-content.xlsx",
})
local pptx = seed_remote_attachment(client, account.account_id, thread.message_id, {
    attachment_id = "extract-real-pptx",
    filename = "known-content.pptx",
    mime_type = "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    fixture = "known-content.pptx",
})
local zipbomb = seed_remote_attachment(client, account.account_id, thread.message_id, {
    attachment_id = "extract-real-zipbomb",
    filename = "zipbomb-shaped.docx",
    mime_type = "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    fixture = "zipbomb-shaped.docx",
})

local pdf_indexed =
    wait_for_attachment_status(client, account.account_id, pdf.attachment_id, "indexed", 30)
harness.assert(pdf_indexed ~= nil, "PDF fixture did not index")
harness.assert(
    string.find(pdf_indexed.extracted_text, "pdfrealfixture", 1, true) ~= nil,
    "PDF extracted text missing fixture token"
)

local docx_indexed =
    wait_for_attachment_status(client, account.account_id, docx.attachment_id, "indexed", 30)
harness.assert(docx_indexed ~= nil, "DOCX fixture did not index")
harness.assert(
    string.find(docx_indexed.extracted_text, "docxrealfixture", 1, true) ~= nil,
    "DOCX extracted text missing fixture token"
)

local xlsx_indexed =
    wait_for_attachment_status(client, account.account_id, xlsx.attachment_id, "indexed", 30)
harness.assert(xlsx_indexed ~= nil, "XLSX fixture did not index")
harness.assert(
    string.find(xlsx_indexed.extracted_text, "xlsxrealfixture", 1, true) ~= nil,
    "XLSX extracted text missing fixture token"
)

local pptx_indexed =
    wait_for_attachment_status(client, account.account_id, pptx.attachment_id, "indexed", 30)
harness.assert(pptx_indexed ~= nil, "PPTX fixture did not index")
harness.assert(
    string.find(pptx_indexed.extracted_text, "pptxrealfixture", 1, true) ~= nil,
    "PPTX extracted text missing fixture token"
)

local zipbomb_skipped = wait_for_attachment_status(
    client,
    account.account_id,
    zipbomb.attachment_id,
    "skipped:zipbomb",
    30
)
harness.assert(zipbomb_skipped ~= nil, "zipbomb-shaped fixture was not skipped")

local pdf_match = wait_for_attachment_match(
    client,
    account.account_id,
    thread.message_id,
    pdf.attachment_id,
    "pdfrealfixture",
    30
)
harness.assert(pdf_match ~= nil, "PDF fixture search match missing")
harness.assert_eq(pdf_match.match_kind.filename, "known-content.pdf", "PDF match filename")

local docx_match = wait_for_attachment_match(
    client,
    account.account_id,
    thread.message_id,
    docx.attachment_id,
    "docxrealfixture",
    30
)
harness.assert(docx_match ~= nil, "DOCX fixture search match missing")
harness.assert_eq(docx_match.match_kind.filename, "known-content.docx", "DOCX match filename")

local xlsx_match = wait_for_attachment_match(
    client,
    account.account_id,
    thread.message_id,
    xlsx.attachment_id,
    "xlsxrealfixture",
    30
)
harness.assert(xlsx_match ~= nil, "XLSX fixture search match missing")
harness.assert_eq(xlsx_match.match_kind.filename, "known-content.xlsx", "XLSX match filename")

local pptx_match = wait_for_attachment_match(
    client,
    account.account_id,
    thread.message_id,
    pptx.attachment_id,
    "pptxrealfixture",
    30
)
harness.assert(pptx_match ~= nil, "PPTX fixture search match missing")
harness.assert_eq(pptx_match.match_kind.filename, "known-content.pptx", "PPTX match filename")

local status, status_err = client:request("ExtractStatus")
harness.assert(status_err == nil, "extract.status failed")
harness.assert_eq(status.queue_depth, 0, "queue depth")
harness.assert(status.indexed_total >= 4, "indexed_total did not include all indexed fixtures")
harness.assert_eq(status.failed_total, 0, "failed total")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
