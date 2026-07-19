-- description: IMAP attachment prefetch serves cache-miss bytes over the resident engine with no dead per-batch session
-- expected: pass
-- fixture: imap-attach-multi.toml
-- protocol: imap
-- ceiling: 90s

-- B9 rewire: the attachment byte source moved onto the resident bifrost
-- engine (`AttachmentByteSource` in
-- `crates/service/src/bifrost/attachment.rs`). The prefetch worker no
-- longer opens a dedicated IMAP session per (account, folder) batch -
-- that pre-B9 session (one LOGIN + one SELECT, then
-- `fetch_attachment_on_selected` per item) became inert once the bytes
-- started flowing through the engine, leaving a LOGIN + SELECT
-- connection that fetched nothing (the regression this gate now guards
-- against). Instead `process_imap_batch` groups the folder's items by
-- message and drives each attachment through the resident engine's
-- `open_raw_rfc822`, hydrating each message's RFC822 at most once and
-- extracting every one of its parts from the single parse.
--
-- Fixture has three single-attachment messages in INBOX. After sync the
-- prefetch worker enqueues all three NULL-hash rows in one folder batch
-- and drains them through the engine. We assert:
--
--   1. All three attachments land a content_hash - the cache-miss path
--      fetched real bytes end-to-end through the engine.
--   2. prefetch.completed reports at least three fetched.
--   3. NO dead prefetch session survives: no IMAP connection has the
--      inert signature the old batch path left behind - a LOGIN plus a
--      mailbox SELECT/EXAMINE but zero body UID FETCHes and zero
--      structural commands (LIST / UID SEARCH). A resident engine
--      connection that serves an attachment always issues a body
--      UID FETCH; a sync connection issues LIST / UID SEARCH. Only the
--      abandoned per-batch session matched login+select+nothing.

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

local function count_mailbox_selections(requests, connection_id)
    return count_commands(requests, connection_id, "SELECT")
        + count_commands(requests, connection_id, "EXAMINE")
end

local function count_body_fetches(requests, connection_id)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == "imap"
            and request.connection_id == connection_id
            and request.command == "UID FETCH"
            and request.detail ~= nil
            and request.detail.body == true
        then
            count = count + 1
        end
    end
    return count
end

local function commands_for_connection(requests, connection_id)
    local commands = {}
    for _, request in ipairs(requests) do
        if request.protocol == "imap"
            and request.connection_id == connection_id
        then
            commands[#commands + 1] = request.command or "<nil>"
        end
    end
    return table.concat(commands, ",")
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

local _, set_err = client:request("SettingsSet", {
    values = {
        { type = "SyncPeriodDays", value = "365" },
    },
})
harness.assert(set_err == nil, "settings.set failed")

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

-- Collect every IMAP connection_id that appears.
local connection_ids = {}
local seen = {}
for _, request in ipairs(requests) do
    if request.protocol == "imap" and request.connection_id ~= nil then
        if not seen[request.connection_id] then
            seen[request.connection_id] = true
            connection_ids[#connection_ids + 1] = request.connection_id
        end
    end
end

-- Regression guard: no connection carries the dead per-batch session's
-- signature - a LOGIN plus a mailbox selection that then does NOTHING
-- useful: zero body UID FETCHes, zero structural LIST / UID SEARCH, and
-- no push IDLE / keepalive NOOP (so a legitimate push or idle-pool
-- connection is not misread as the abandoned prefetch session). That
-- "log in, select, and stop" shape is exactly the inert session the B9
-- rewire removed when the byte source moved onto the resident engine.
local dead_session = nil
for _, cid in ipairs(connection_ids) do
    local login_count = count_commands(requests, cid, "LOGIN")
    local selection_count = count_mailbox_selections(requests, cid)
    local body_fetches = count_body_fetches(requests, cid)
    local list_count = count_commands(requests, cid, "LIST")
    local search_count = count_commands(requests, cid, "UID SEARCH")
    local idle_count = count_commands(requests, cid, "IDLE")
    local noop_count = count_commands(requests, cid, "NOOP")
    if login_count >= 1
        and selection_count >= 1
        and body_fetches == 0
        and list_count == 0
        and search_count == 0
        and idle_count == 0
        and noop_count == 0
    then
        dead_session = cid
        break
    end
end

harness.assert(dead_session == nil,
    "found an inert prefetch IMAP session (login+select, no fetch/list/search) on connection_id=" ..
    tostring(dead_session) ..
    " - the B9 rewire must not leave a dedicated attachment-prefetch session; commands=" ..
    (dead_session ~= nil and commands_for_connection(requests, dead_session) or ""))

-- Positive check: the attachment bytes were served over the engine's
-- IMAP connection(s) as whole-message body fetches. There must be at
-- least three body UID FETCHes beyond the ones sync issued for message
-- bodies (three messages -> three attachment RFC822 hydrations).
local total_body_fetches = 0
for _, cid in ipairs(connection_ids) do
    total_body_fetches = total_body_fetches + count_body_fetches(requests, cid)
end
harness.assert(total_body_fetches >= 3,
    "expected at least three IMAP body fetches serving the three attachments (got " ..
    tostring(total_body_fetches) .. ")")

harness.write_summary({
    correct = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
    imap_connections = #connection_ids,
    total_body_fetches = total_body_fetches,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
