-- description: account.delete tombstones the deleted account's unshared blobs (Phase 4 deferral)
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 60s

-- Phase 4 of the attachments roadmap added
-- `AccountDeletionStep::AttachmentCache` which calls
-- `PackStore::tombstone` on every blob whose only referencing account
-- is the one being deleted. Phase 8c adds end-to-end harness
-- coverage: seed, sync, capture content_hash, delete account, query
-- `attachment_blobs` directly (the row outlives the cascade-deleted
-- `attachments` row) and assert `tombstoned_at IS NOT NULL`.
--
-- Two-account shared-blob handling (B still references -> A's
-- delete leaves blob live) is verified by Phase 4 code review;
-- harness coverage there waits for cross-account fixture
-- machinery.

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    if #attachments == 1 then
        return attachments[1]
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

local dir = harness.data_dir("sync_jmap_account_delete_tombstones")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-account-delete@example.test",
    display_name = "Sync JMAP Account Delete",
    account_name = "Sync JMAP Account Delete",
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

-- Prefetch caches the blob; the post-sync sweep enqueues, the worker
-- writes to PackStore, prefetch.completed fires.
local prefetch = wait_for_method(queue, "prefetch.completed", 30)
harness.assert(prefetch ~= nil, "prefetch.completed not observed")
harness.assert((prefetch.fetched or 0) >= 1, "prefetch did not fetch")

-- Capture the content_hash now - it won't be readable through
-- TestQueryDbState after the cascade-delete drops the attachments
-- row.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local row = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(row ~= nil, "sample.txt row missing")
harness.assert(row.content_hash ~= nil, "content_hash should be populated pre-delete")
local hash = row.content_hash

-- Pre-delete sanity: blob is live.
local pre, pre_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(pre_err == nil, "pre-delete probe failed")
harness.assert(pre.present, "blob should be present in attachment_blobs pre-delete")
harness.assert(pre.tombstoned_at == nil, "blob should be live pre-delete")

-- Delete the account. AccountDeletionStep::AttachmentCache (Phase 4)
-- runs synchronously inside this dispatch and tombstones every
-- unshared blob.
harness.marker("ACCOUNT_DELETE")
local _, delete_err = client:request("AccountDelete", {
    account_id = account.account_id,
})
harness.assert(delete_err == nil, "account.delete failed: " .. tostring(delete_err))

-- Post-delete: the attachments row cascade-deleted, but the
-- attachment_blobs row survives with tombstoned_at populated.
local post, post_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(post_err == nil, "post-delete probe failed")
harness.assert(post.present, "attachment_blobs row should survive account delete")
harness.assert(post.tombstoned_at ~= nil,
    "blob should be tombstoned after account delete (got tombstoned_at=" .. tostring(post.tombstoned_at) .. ")")

harness.write_summary({
    correct = 1,
    tombstoned_at = post.tombstoned_at,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
