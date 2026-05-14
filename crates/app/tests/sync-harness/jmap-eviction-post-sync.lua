-- description: Post-sync eviction sweep fires after every sync and emits eviction.completed
-- expected: pass
-- fixture: jmap-attach-ancient.toml
-- protocol: jmap
-- ceiling: 60s

-- Phase 8a (attachments roadmap): every sync completion runs an
-- eviction sweep right after the post-sync prefetch sweep. Even when
-- there's nothing to tombstone, the sweep emits eviction.completed
-- { trigger = "post_sync" } so harness scripts (and future UI hooks)
-- have a deterministic signal. The fixture's message is dated 2020,
-- so prefetch skips it (out of window) and eviction has nothing to
-- evict (no attachment_blobs row exists yet); the test asserts the
-- sweep ran and reported zero evictions, not that eviction did
-- something.

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

local dir = harness.data_dir("sync_jmap_eviction_post_sync")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-eviction-post-sync@example.test",
    display_name = "Sync JMAP Eviction Post-Sync",
    account_name = "Sync JMAP Eviction Post-Sync",
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

harness.marker("EVICTION_WAIT")
local eviction = wait_for_eviction_trigger(queue, "post_sync", 15)
harness.assert(eviction ~= nil, "eviction.completed { trigger = post_sync } not observed after sync")
harness.assert_eq(eviction.blobs_tombstoned, 0, "ancient message has no cached blob yet, so nothing to tombstone")
harness.assert_eq(eviction.superseded, false, "post-sync sweep should not be superseded")

harness.write_summary({
    correct = 1,
    blobs_tombstoned = eviction.blobs_tombstoned,
    pages_walked = eviction.pages_walked,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
