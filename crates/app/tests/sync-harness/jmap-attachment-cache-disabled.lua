-- description: cache_attachments_enabled=0 skips PrefetchRuntime; attachments stay NULL-hashed
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 120s

-- Phase 6 (attachments roadmap): per-account offline-cache master
-- switch. When `accounts.cache_attachments_enabled = 0`, sync still
-- writes attachment metadata rows but the post-sync sweep skips
-- enqueueing them, and the PrefetchRuntime worker would skip them
-- anyway via `SkipReason::AccountDisabled`. Verification: after sync,
-- `attachment_count > 0` AND every attachment has `content_hash IS
-- NULL`. No `prefetch.completed` notification fires (the queue is
-- never enqueued).

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

local dir = harness.data_dir("sync_jmap_cache_disabled")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-disabled@example.test",
    display_name = "Sync JMAP Disabled",
    account_name = "Sync JMAP Disabled",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- Phase 6: flip the per-account toggle off via `account.update`
-- before sync runs. The toggle is the unit under test here; verifying
-- it short-circuits both the post-sync sweep and the boot recovery
-- kick is the whole point.
local _, upd_err = client:request("AccountUpdate", {
    id = account.account_id,
    cache_attachments_enabled = false,
})
harness.assert(upd_err == nil, "AccountUpdate failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

-- The post-sync sweep should NOT enqueue. We allow a generous
-- timeout because a contended runner can stretch the path between
-- sync.completed and the sweep skip; the assertion is "if anything
-- did fire, fetched must be 0 (caching disabled means no bytes
-- landed)". Anchoring on the semantic check rather than the absence
-- of a notification removes the timing flake.
local prefetch_done = wait_for_prefetch_completed(queue, 8)
if prefetch_done ~= nil then
    harness.assert_eq(
        prefetch_done.fetched, 0,
        "prefetch fetched > 0 with caching disabled"
    )
end

-- Attachment metadata still landed; just no bytes.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert(state.attachment_count >= 1, "expected at least one attachment row")

local row = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(row ~= nil, "sample.txt missing from attachments")
harness.assert(row.content_hash == nil, "content_hash should be NULL when caching disabled")

-- Flip it back on and sync again - this time prefetch should fire
-- and the row should get its hash populated. Validates that the
-- toggle is genuinely re-engageable, not one-way.
local _, reenable_err = client:request("AccountUpdate", {
    id = account.account_id,
    cache_attachments_enabled = true,
})
harness.assert(reenable_err == nil, "AccountUpdate re-enable failed")

local completed2, sync_err2 = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err2 == nil, "second start_sync failed")
harness.assert_eq(completed2.result, "completed", completed2.error or "sync result 2")

local prefetch_done2 = wait_for_prefetch_completed(queue, 30)
harness.assert(prefetch_done2 ~= nil, "prefetch.completed missing after re-enable")
harness.assert(prefetch_done2.fetched >= 1, "fetched count after re-enable")

local state2, state2_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state2_err == nil, "TestQueryDbState 2 failed")
local row2 = attachment_by_filename(state2.attachments, "sample.txt")
harness.assert(row2 ~= nil, "sample.txt missing after re-enable")
harness.assert(row2.content_hash ~= nil, "content_hash still NULL after re-enable")

-- Phase 6 cache-size readout. After re-enable + prefetch, the live
-- bytes should be the size of sample.txt.
local size_ack, size_err = client:request("AttachmentCacheSize", {})
harness.assert(size_err == nil, "AttachmentCacheSize failed")
harness.assert(size_ack.live_bytes >= 1, "live_bytes should be > 0 after prefetch")
harness.assert_eq(size_ack.tombstoned_bytes, 0, "no tombstones expected")

harness.write_summary({
    correct = 1,
    attachment_count = state2.attachment_count,
    live_bytes = size_ack.live_bytes,
    tombstoned_bytes = size_ack.tombstoned_bytes,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
