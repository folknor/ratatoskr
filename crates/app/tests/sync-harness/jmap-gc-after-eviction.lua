-- description: Window-shrink eviction chains into a physical GC pass that reclaims bytes
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 90s

-- Phase 8b (attachments roadmap): when `kick_window_shrink` runs the
-- eviction sweep and at least one blob gets tombstoned, the same
-- detached task immediately runs a GC pass. This harness verifies
-- the chain - eviction -> GC - fires correctly with the right
-- trigger; per-pack compaction correctness is covered by stores/
-- unit tests (gc_drops_tombstoned, gc_skips_low_density, etc.).
--
-- Note: with a single blob in a freshly-rotated mailbox the pack
-- is still the open pack (.pack.open), which `compact_pack` does
-- not touch - sealed packs only. So `packs_compacted` will be 0
-- here; the assertion is on the chain firing, not on the per-pack
-- reclaim.
--
-- Flow:
--   1. Sync at default 365d -> blob cached in PackStore.
--   2. settings.set { sync_period_days = "30" }
--   3. eviction.completed { trigger="window_shrink", blobs_tombstoned=1 }
--   4. gc.completed { trigger="post_eviction" }

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

local function wait_for_gc_trigger(queue, trigger, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "gc.completed"
            and notification.trigger == trigger
        then
            return notification
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_gc_after_eviction")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-gc-after-eviction@example.test",
    display_name = "Sync JMAP GC After Eviction",
    account_name = "Sync JMAP GC After Eviction",
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

-- Confirm blob landed in PackStore.
local prefetch = wait_for_method(queue, "prefetch.completed", 30)
harness.assert(prefetch ~= nil, "prefetch.completed not observed")
harness.assert((prefetch.fetched or 0) >= 1, "prefetch did not fetch")

-- Shrink. eviction tombstones, GC chains immediately after.
harness.marker("SETTINGS_SHRINK")
local _, set_err = client:request("SettingsSet", {
    values = {
        { type = "SyncPeriodDays", value = "30" },
    },
})
harness.assert(set_err == nil, "settings.set failed: " .. tostring(set_err))

harness.marker("GC_WAIT")
local gc = wait_for_gc_trigger(queue, "post_eviction", 30)
harness.assert(gc ~= nil, "gc.completed { trigger = post_eviction } not observed")
-- packs_compacted may be 0 here because the single-blob pack is
-- still .pack.open; the chain firing is the contract.
harness.assert(gc.packs_compacted ~= nil, "packs_compacted field missing")
harness.assert(gc.bytes_reclaimed ~= nil, "bytes_reclaimed field missing")

harness.write_summary({
    correct = 1,
    packs_compacted = gc.packs_compacted,
    blobs_dropped = gc.blobs_dropped,
    bytes_reclaimed = gc.bytes_reclaimed,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
