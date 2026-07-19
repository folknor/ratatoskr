-- description: One IMAP message with three attachments hydrates its RFC822 once, not once per attachment
-- expected: pass
-- fixture: imap-attach-onemsg-multi.toml
-- protocol: imap
-- ceiling: 90s

-- B9 finding B (the once-per-message invariant). After the attachment
-- byte source moved onto the resident bifrost engine, a per-item
-- `open_raw_rfc822` rewire would download an N-attachment message's full
-- RFC822 N times. `process_imap_batch` prevents that by grouping the
-- folder batch by message and hydrating each message's RFC822 at most
-- once (`AttachmentByteSource::fetch_imap_rfc822`), extracting every part
-- from the single parse.
--
-- Fixture: ONE INBOX message carrying THREE attachments. After sync,
-- prefetch enqueues all three NULL-hash rows in one folder batch and one
-- message group. We assert:
--
--   1. All three attachments land a content_hash (all parts extracted
--      from the shared RFC822).
--   2. Prefetch issues exactly ONE whole-message body fetch for the
--      three attachments, not three. We isolate prefetch from bifrost's
--      sync-time hydration (which issues its own BODY[]/BODY[TEXT]/
--      BODY[HEADER] fetches) by clearing the mock request log AFTER sync
--      completes, then counting only whole-message `BODY.PEEK[]` fetches
--      (saehrimnir attr `BODY[]`). open_raw_rfc822 issues exactly that;
--      a per-attachment regression would issue three.

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local function wait_for_prefetch_completed(queue, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "prefetch.completed"
        then
            return notification
        end
    end
    return nil
end

local function wait_for_all_content_hashes(client, account_id, filenames, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
            attachment_limit = 10,
        })
        harness.assert(state_err == nil, "TestQueryDbState failed")
        local all_populated = true
        for _, filename in ipairs(filenames) do
            local row = attachment_by_filename(state.attachments, filename)
            if row == nil or row.content_hash == nil then
                all_populated = false
                break
            end
        end
        if all_populated then
            return state
        end
        harness.sleep(250)
    end
    return nil
end

-- A whole-message fetch is `BODY.PEEK[]` (saehrimnir attr `BODY[]`),
-- which is exactly what open_raw_rfc822 issues. Sync-time hydration uses
-- BODY[TEXT] / BODY[HEADER] / part fetches, which we deliberately do not
-- count - only whole-message downloads matter for once-per-message.
local function attrs_contain(detail, name)
    if detail == nil or detail.attrs == nil then
        return false
    end
    for _, attr in ipairs(detail.attrs) do
        if attr == name then
            return true
        end
    end
    return false
end

local function count_whole_message_fetches(requests)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == "imap"
            and request.command == "UID FETCH"
            and attrs_contain(request.detail, "BODY[]")
        then
            count = count + 1
        end
    end
    return count
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_attach_onemsg_multi")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local _, set_err = client:request("SettingsSet", {
    values = {
        { type = "SyncPeriodDays", value = "365" },
    },
})
harness.assert(set_err == nil, "settings.set failed")

local account, account_err = client:request("TestSeedAccount", {
    email = "imap-onemsg@example.test",
    display_name = "IMAP One Message Multi",
    account_name = "IMAP One Message Multi",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- Isolate prefetch from sync-time hydration: sync has completed, so
-- clearing the request log now leaves only the prefetch worker's
-- subsequent whole-message fetches to count. (Clearing does not touch
-- live connections - bifrost's resident IMAP connection stays up and its
-- next fetches are logged fresh.)
harness.clear_mock_requests(admin_endpoint)

harness.marker("PREFETCH_WAIT_START")
local prefetch_done = wait_for_prefetch_completed(queue, 30)
harness.marker("PREFETCH_WAIT_END")
harness.assert(prefetch_done ~= nil, "prefetch.completed not observed")
harness.assert((prefetch_done.fetched or 0) >= 3,
    "expected prefetch to fetch at least 3 attachments, got " ..
    tostring(prefetch_done.fetched))

local state = wait_for_all_content_hashes(
    client, account.account_id,
    { "first.txt", "second.txt", "third.txt" }, 10)
harness.assert(state ~= nil,
    "not all three attachments of the single message had content_hash populated")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local whole_message_fetches = count_whole_message_fetches(requests)

-- One message, three attachments, hydrated once. With sync excluded (log
-- cleared after SYNC_END), the prefetch worker's whole-message download
-- count must be 1 - the single shared open_raw_rfc822. A per-attachment
-- regression would issue three. `<= 1` also tolerates the benign race
-- where a prefetch fetch landed just before the post-sync clear (it can
-- only lower the count, never inflate it past a genuine regression).
harness.assert(whole_message_fetches <= 1,
    "expected exactly one shared whole-message prefetch download for the " ..
    "three attachments, got " .. tostring(whole_message_fetches) ..
    " - three attachments must not trigger three full-message downloads")

harness.write_summary({
    correct = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
    prefetch_whole_message_fetches = whole_message_fetches,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
