-- description: Window-shrink fires eviction sweep that tombstones now-out-of-window blobs
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 90s

-- Phase 8a (attachments roadmap): when the user shrinks
-- sync_period_days, an eviction sweep runs against the freshly-
-- narrowed window and tombstones every blob whose every referencing
-- message is now out-of-window. The jmap-attach fixture has its
-- message at 2026-01-15 (~120d before today's 2026-05-14), inside
-- the default 365d window but outside a 30d shrink target.
--
-- Flow:
--   1. Sync with default 365d -> prefetch caches the blob ->
--      content_hash populated.
--   2. settings.set { sync_period_days = "30" }
--   3. window-shrink trigger fires eviction sweep with the new
--      window; the cached blob is now out-of-window.
--   4. eviction.completed { trigger = "window_shrink",
--      blobs_tombstoned = 1 } observed.

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

local function wait_for_eviction_trigger(queue, trigger, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "eviction.completed"
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

local dir = harness.data_dir("sync_jmap_eviction_window_shrink")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-eviction-shrink@example.test",
    display_name = "Sync JMAP Eviction Shrink",
    account_name = "Sync JMAP Eviction Shrink",
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

-- Wait for prefetch to cache the blob.
local prefetch = wait_for_method(queue, "prefetch.completed", 30)
harness.assert(prefetch ~= nil, "prefetch.completed not observed")
harness.assert((prefetch.fetched or 0) >= 1, "prefetch fetched count")

-- Verify the row has a content_hash; that confirms PackStore::put
-- ran and attachment_blobs has a live row to evict.
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
local row = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(row ~= nil, "sample.txt row missing")
harness.assert(row.content_hash ~= nil, "content_hash should be populated before shrink")

-- Shrink. The detached kick_window_shrink runs the eviction sweep
-- with window_start_unix = now - 30 days, well after our 2026-01-15
-- message date.
harness.marker("SETTINGS_SET")
local _, set_err = client:request("SettingsSet", {
    values = {
        { type = "SyncPeriodDays", value = "30" },
    },
})
harness.assert(set_err == nil, "settings.set failed: " .. tostring(set_err))

harness.marker("EVICTION_WAIT")
local eviction = wait_for_eviction_trigger(queue, "window_shrink", 30)
harness.assert(eviction ~= nil, "eviction.completed { trigger = window_shrink } not observed")
harness.assert((eviction.blobs_tombstoned or 0) >= 1,
    "expected >= 1 blob tombstoned, got " .. tostring(eviction.blobs_tombstoned))
harness.assert_eq(eviction.superseded, false, "single shrink should not be superseded")

harness.write_summary({
    correct = 1,
    blobs_tombstoned = eviction.blobs_tombstoned,
    pages_walked = eviction.pages_walked,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
