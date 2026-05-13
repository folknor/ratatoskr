-- description: JMAP sync triggers PrefetchRuntime; attachment bytes land in PackStore before user click
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 120s

-- Phase 4 (attachments roadmap) end-to-end. The mock JMAP server serves
-- one attachment (sample.txt). After sync, the post-sync sweep in
-- `run_sync` enqueues the NULL-hash attachment on PrefetchRuntime's
-- Sync priority lane; the worker fetches the bytes through the same
-- provider call as `attachment.fetch` cache-miss and writes them to
-- PackStore. Verification: `attachments.content_hash` is populated and
-- a pack file exists on disk.

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

local function wait_for_content_hash(client, account_id, timeout_s)
    -- Backstop poll: the notification path is primary, but the DB row
    -- update is the actual correctness signal. If a regression broke
    -- PrefetchCompleted emission, this would still surface a hash.
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
            attachment_limit = 10,
        })
        harness.assert(state_err == nil, "TestQueryDbState failed")
        local row = attachment_by_filename(state.attachments, "sample.txt")
        if row ~= nil and row.content_hash ~= nil then
            return row
        end
        harness.sleep(250)
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_attachment_prefetch")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-prefetch@example.test",
    display_name = "Sync JMAP Prefetch",
    account_name = "Sync JMAP Prefetch",
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

-- After sync completes, the post-sync sweep has enqueued the
-- attachment onto PrefetchRuntime. Wait for the queue to drain.
harness.marker("PREFETCH_WAIT_START")
local prefetch_done = wait_for_prefetch_completed(queue, 30)
harness.marker("PREFETCH_WAIT_END")
harness.assert(prefetch_done ~= nil, "prefetch.completed not observed")
harness.assert((prefetch_done.fetched or 0) >= 1, "prefetch fetched count")
harness.assert_eq(prefetch_done.failed, 0, "prefetch failed count")

-- The notification is the primary signal; the DB row is the
-- correctness contract. Re-check directly.
local row = wait_for_content_hash(client, account.account_id, 5)
harness.assert(row ~= nil, "attachment row missing content_hash after prefetch")
harness.assert(row.content_hash ~= nil, "content_hash nil")
harness.assert(#row.content_hash == 64, "content_hash should be 32-byte BLAKE3 hex (64 chars)")

-- Pack file existence: PackStore writes data-NNNNNN.pack[.open] under
-- <data_dir>/attachment_packs/.
local packs_dir = dir .. "/attachment_packs"
harness.assert(harness.path_exists(packs_dir), "attachment_packs dir missing")
harness.assert(
    harness.dir_has_prefix(packs_dir, "data-"),
    "no pack file under attachment_packs/"
)

-- The fact that prefetch.completed fired with fetched=1 already proves
-- the provider was hit (PackStore::put would not have run otherwise).
-- We log the saehrimnir request count for telemetry.
local requests = harness.mock_requests(admin_endpoint, { stable = true })

harness.write_summary({
    correct = 1,
    attachment_count = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
    provider_requests = #requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
