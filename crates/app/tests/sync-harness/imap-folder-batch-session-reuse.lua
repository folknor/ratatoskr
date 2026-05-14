-- description: IMAP attachment prefetch batches three attachments under one (LOGIN + SELECT) session per (account, folder)
-- expected: pass
-- fixture: imap-attach-multi.toml
-- protocol: imap
-- ceiling: 90s

-- Phase 7 of the attachments roadmap shipped folder-batched IMAP
-- attachment fetches (`process_imap_batch` in
-- `crates/service/src/prefetch_imap.rs`): one LOGIN + one SELECT per
-- `(account_id, folder)` group, then `fetch_attachment_on_selected`
-- per item under the shared session. The earlier
-- `imap-attachment-prefetch.lua` test could only verify content_hash
-- population because saehrimnir's RequestLog had no way to tell two
-- IMAP sessions apart. Saehrimnir now tags every `RequestEntry` with
-- a process-wide `connection_id` (rewritten to 0-based dense indices
-- under `?stable=true`), which lets the harness group commands by
-- session.
--
-- Fixture has three attachments in INBOX. After sync, the prefetch
-- worker enqueues all three NULL-hash rows and processes them in one
-- IMAP batch. We:
--
--   1. Find every IMAP connection_id that issued any UID FETCH
--      against a body part (sync fetches bodies for email content,
--      prefetch fetches BODY[part] for attachments - both share the
--      `request.detail.body == true` signal in saehrimnir).
--   2. Pick the connection_id with the most body fetches. With one
--      sync + three prefetched attachments and folder-batching
--      working, that is the prefetch session with three fetches.
--      Regression (one session per attachment) would split those
--      three fetches across three distinct connection_ids, none of
--      which has more than one.
--   3. Assert the chosen connection issued exactly one LOGIN and
--      exactly one SELECT - the Phase 7 contract.

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

local function count_commands(requests, connection_id, command)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == "imap"
            and request.connection_id == connection_id
            and request.command == command
        then
            count = count + 1
        end
    end
    return count
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_folder_batch_session_reuse")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "imap-multi@example.test",
    display_name = "IMAP Folder Batch",
    account_name = "IMAP Folder Batch",
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
harness.assert(state ~= nil, "not all attachments had content_hash populated after prefetch")

local requests = harness.mock_requests(admin_endpoint, { stable = true })

-- Group body fetches by connection_id. Sync also fetches bodies (for
-- email content); prefetch fetches BODY[part] for attachments. Both
-- show as UID FETCH with detail.body == true, so we pick the session
-- with the most body fetches as the prefetch one.
local body_fetches_by_conn = {}
for _, request in ipairs(requests) do
    if request.protocol == "imap"
        and request.command == "UID FETCH"
        and request.detail ~= nil
        and request.detail.body == true
        and request.connection_id ~= nil
    then
        local cid = request.connection_id
        body_fetches_by_conn[cid] = (body_fetches_by_conn[cid] or 0) + 1
    end
end

local prefetch_conn = nil
local prefetch_fetch_count = 0
for cid, count in pairs(body_fetches_by_conn) do
    if count > prefetch_fetch_count then
        prefetch_conn = cid
        prefetch_fetch_count = count
    end
end

harness.assert(prefetch_conn ~= nil,
    "no IMAP connection issued any body fetches - prefetch path never ran")
harness.assert(prefetch_fetch_count >= 3,
    "prefetch session should batch all three attachment fetches into one connection " ..
    "(got " .. tostring(prefetch_fetch_count) .. " body fetches on connection_id=" ..
    tostring(prefetch_conn) .. " - a regression to one-session-per-attachment would " ..
    "spread these across three connection_ids)")

local login_count = count_commands(requests, prefetch_conn, "LOGIN")
local select_count = count_commands(requests, prefetch_conn, "SELECT")

harness.assert_eq(login_count, 1,
    "prefetch session should issue exactly one LOGIN (got " ..
    tostring(login_count) .. ")")
harness.assert_eq(select_count, 1,
    "prefetch session should issue exactly one SELECT (got " ..
    tostring(select_count) .. ")")

harness.write_summary({
    correct = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
    prefetch_connection_id = prefetch_conn,
    prefetch_body_fetches = prefetch_fetch_count,
    prefetch_logins = login_count,
    prefetch_selects = select_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
