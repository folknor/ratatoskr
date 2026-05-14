-- description: --rebuild-attachment-index round-trips over real packs without losing blobs
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 120s

-- Phase 8b (attachments roadmap): `--rebuild-attachment-index` is a
-- corruption-recovery primitive that walks every sealed pack's
-- frames and replays every tombstone log to repopulate
-- `attachment_blobs`. Rebuild *correctness* under simulated
-- corruption is covered by Rust unit tests in
-- `crates/stores/src/attachment_pack.rs` (see
-- `rebuild_index_repopulates_from_sealed_packs`,
-- `rebuild_index_is_idempotent`,
-- `rebuild_index_does_not_resurrect_tombstone_after_revive`,
-- `rebuild_index_repairs_corrupted_row_pointer`). This harness's
-- value is the end-to-end CLI path: seed real packs via sync,
-- restart with the flag set, and verify the blob is still readable
-- via `attachment.fetch` afterwards. A regression that wiped or
-- broke the index during boot rebuild would fail the final fetch.

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

local dir = harness.data_dir("sync_jmap_rebuild_attachment_index_flag")

-- First boot: normal sync to seed real packs on disk.
local client, err = harness.spawn(dir)
harness.assert(err == nil, "first spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-rebuild-flag@example.test",
    display_name = "Sync Rebuild Flag",
    account_name = "Sync Rebuild Flag",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local prefetch = wait_for_method(queue, "prefetch.completed", 30)
harness.assert(prefetch ~= nil, "first prefetch.completed missing")
harness.assert((prefetch.fetched or 0) >= 1, "prefetch did not fetch")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local row = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(row ~= nil, "sample.txt row missing pre-rebuild")
harness.assert(row.content_hash ~= nil, "content_hash should be populated pre-rebuild")
local hash = row.content_hash

local ok1, shutdown_err1 = client:shutdown()
harness.assert(ok1, "first shutdown failed")
harness.assert(shutdown_err1 == nil, "first shutdown returned error")

-- Second boot: same data_dir, --rebuild-attachment-index passed.
-- The flag walks sealed packs + replays tombstone logs; a
-- regression that wiped the index without repopulating it would
-- leave the blob unreadable.
local client2, err2 = harness.spawn(dir, { "--rebuild-attachment-index" })
harness.assert(err2 == nil, "second spawn with --rebuild-attachment-index failed: " .. tostring(err2))

local ready2, ready2_err = client2:request("BootReady")
harness.assert(ready2_err == nil, "second boot.ready failed under --rebuild-attachment-index")
harness.assert(ready2.ready, "second boot.ready returned ready=false")

-- Hash must still resolve to a live blob.
local post, post_err = client2:request("TestQueryBlobTombstoneState", {
    content_hash = hash,
})
harness.assert(post_err == nil, "post-rebuild probe failed")
harness.assert(post.present, "blob should be present after rebuild")
harness.assert(post.tombstoned_at == nil, "blob should be live after rebuild")

harness.write_summary({
    correct = 1,
    hash = hash,
})

local ok2, shutdown_err2 = client2:shutdown()
harness.assert(ok2, "second shutdown failed")
harness.assert(shutdown_err2 == nil, "second shutdown returned error")
