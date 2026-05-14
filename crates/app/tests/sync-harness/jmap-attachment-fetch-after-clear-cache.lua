-- description: attachment.fetch after clear-cache refetches the blob (CRIT-1)
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 120s

-- Outside-review CRIT-1: a row with content_hash set whose
-- corresponding attachment_blobs row is tombstoned (by eviction or
-- clear-cache) used to fail attachment.fetch with "blob indexed in
-- attachments but absent from pack store" - the cache-hit branch
-- returned early on content_hash being set, and PackStore::get
-- returned None for tombstoned blobs.
--
-- The fix: handle_fetch's cache-hit branch now treats tombstoned
-- blobs as misses and falls through to the provider re-fetch path,
-- which revives the blob via PackStore::put's tombstone-revive
-- branch. This harness asserts the end-to-end refetch succeeds and
-- the blob ends up live again.

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local function wait_for_method(queue, method, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil and notification.method == method then
            return notification
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_attachment_fetch_after_clear")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-fetch-after-clear@example.test",
    display_name = "Sync Fetch After Clear",
    account_name = "Sync Fetch After Clear",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- Wait for prefetch to land bytes.
local prefetch = wait_for_method(queue, "prefetch.completed", 30)
harness.assert(prefetch ~= nil, "prefetch.completed missing")
harness.assert((prefetch.fetched or 0) >= 1, "prefetch did not fetch")

-- Capture row + hash pre-clear.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local row = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(row ~= nil, "sample.txt row missing")
harness.assert(row.content_hash ~= nil, "content_hash should be populated pre-clear")
local hash = row.content_hash
local attachment_id = row.id
local message_id = row.message_id

-- Pre-clear sanity: blob live.
local pre, pre_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(pre_err == nil, "pre-clear probe failed")
harness.assert(pre.present, "blob should be present pre-clear")
harness.assert(pre.tombstoned_at == nil, "blob should be live pre-clear")

-- Clear the cache: tombstones every blob.
local _clear, clear_err = client:request("AttachmentClearCache", {})
harness.assert(clear_err == nil, "attachment.clear_cache failed: " .. tostring(clear_err))

-- Post-clear: blob tombstoned.
local post_clear, post_clear_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(post_clear_err == nil, "post-clear probe failed")
-- The blob row may have been physically GCed by the clear-cache
-- handler's chained GC pass, in which case present=false. Either
-- "present + tombstoned" or "absent" is a valid post-clear state.
if post_clear.present then
    harness.assert(
        post_clear.tombstoned_at ~= nil,
        "blob should be tombstoned post-clear if still present"
    )
end

-- The critical assertion: fetch must succeed even though the blob
-- was tombstoned. Pre-fix this returned an "absent from pack store"
-- internal error; post-fix it re-fetches from the provider and
-- revives via PackStore::put.
local fetch, fetch_err = client:request("AttachmentFetch", {
    account_id = account.account_id,
    message_id = message_id,
    attachment_id = attachment_id,
})
harness.assert(
    fetch_err == nil,
    "attachment.fetch after clear-cache failed: " .. tostring(fetch_err)
)
harness.assert(fetch.relative_path ~= nil, "fetch ack missing relative_path")
harness.assert(#fetch.content_hash == 64, "fetch ack hash should be 64-char hex")

-- Post-fetch: blob revived (live again).
local post_fetch, post_fetch_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(post_fetch_err == nil, "post-fetch probe failed")
harness.assert(post_fetch.present, "blob should be present after refetch")
harness.assert(
    post_fetch.tombstoned_at == nil,
    "blob should be live after refetch (got tombstoned_at=" .. tostring(post_fetch.tombstoned_at) .. ")"
)

harness.write_summary({
    correct = 1,
    hash = hash,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
