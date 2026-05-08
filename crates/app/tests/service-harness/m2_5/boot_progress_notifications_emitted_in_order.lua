-- description: boot.progress notifications arrive in canonical phase order
-- ceiling: 60s

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local expected = {
    "LoadingKey",
    "OpeningDatabase",
    "Migrating",
    "RecoveringPendingOps",
    "SweepingQueuedDrafts",
    "BackfillingThreadParticipants",
    "DrainingDraftWal",
    "OpeningBodyAndInlineStores",
    "OpeningSearchIndex",
    "RunningInvariantPass",
}

local dir = harness.data_dir("m2_5_boot_progress_order")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "first event")
local queue = first.client:notifications()

local count = 0
local last = nil
while count < #expected do
    local notification = queue:recv(10)
    harness.assert(notification ~= nil, "timed out waiting for boot.progress")
    harness.assert_eq(notification.type, "BootProgress", "notification type")

    local phase = notification.phase_kind
    if phase ~= last then
        count = count + 1
        harness.assert_eq(phase, expected[count], "boot phase order")
        last = phase
    end
end

local second = next_lifecycle(events, 15)
harness.assert_eq(second.type, "BootReady", "second event")
harness.assert(second.response.ready, "BootReady response not ready")

local ok, shutdown_err = first.client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
