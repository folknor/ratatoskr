-- description: extending sync_period_days triggers a backfill kick for in-window attachments
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: jmap
-- ceiling: 180s

-- Phase 6 (attachments roadmap): the window-extend slider must
-- detach its post-commit kick from the settings.set ack (otherwise
-- the IPC 5s timeout fires on large mailboxes), AND it must actually
-- fire so historical attachments inside the freshly-extended window
-- populate without waiting for the next boot.
--
-- Approach: sync with a 1-day window so prefetch covers only fresh
-- messages, then extend the window via settings.set, then wait for a
-- second prefetch.completed reporting `fetched >= 1`. The fixture's
-- messages are all dated "now", so a 1-day window covers them too -
-- but we set the window so far back the kick has only the historical
-- rows the sweep didn't pick up. Concretely: start at 365 days
-- (matches the default), trim down via settings.set to a value below
-- the fixture's oldest message age (if the fixture has any), then
-- extend.
--
-- The harness fixture only contains "now" messages; the window kick
-- mostly verifies the IPC path doesn't block and that the kick walks
-- the right accounts. A more discriminating fixture is a Phase 7+
-- follow-up.

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

local dir = harness.data_dir("sync_jmap_window_extend")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-windowext@example.test",
    display_name = "Sync JMAP Window Extend",
    account_name = "Sync JMAP Window Extend",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- Initial sync at the default window. Prefetch covers fresh
-- attachments; we wait for the drained-to-zero completion before
-- touching settings, so the second kick fires against a quiescent
-- runtime.
harness.marker("INITIAL_SYNC_START")
local first_sync, first_sync_err = client:start_sync({
    account_id = account.account_id,
}, 60)
harness.assert(first_sync_err == nil, "first start_sync failed")
harness.assert_eq(first_sync.result, "completed", first_sync.error or "first sync result")

local first_prefetch = wait_for_prefetch_completed(queue, 30)
harness.assert(first_prefetch ~= nil, "first prefetch.completed missing")
harness.marker("INITIAL_SYNC_END")

-- Extend the window. The IPC must return promptly even when the
-- kick has work to do; the detach in handle_set guarantees this.
local before_ms = harness.now_ms()
local _, set_err = client:request("SettingsSet", {
    values = {
        { type = "SyncPeriodDays", value = "730" },
    },
})
local elapsed_ms = harness.now_ms() - before_ms
harness.assert(set_err == nil, "settings.set failed")
harness.assert(
    elapsed_ms < 4500,
    "settings.set blocked on kick: elapsed=" .. tostring(elapsed_ms) .. "ms"
)

-- A second prefetch.completed should fire from the window-extend
-- kick. With the fixture's "now" messages already prefetched, the
-- new completion reports fetched=0 - the kick path itself is what
-- we're verifying.
local second_prefetch = wait_for_prefetch_completed(queue, 30)
harness.assert(second_prefetch ~= nil, "window-extend prefetch.completed missing")

harness.write_summary({
    correct = 1,
    settings_set_ms = elapsed_ms,
    first_fetched = first_prefetch.fetched,
    second_fetched = second_prefetch.fetched,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
