-- description: notifications tagged with the dying generation never reach a post-respawn drain
-- ceiling: 60s

-- The reader-side gate (reader_should_enqueue) and dispatch-side gate
-- (notification_should_dispatch) are unit-tested in
-- crates/app/src/service_client.rs. This script runs the FULL pipeline:
-- reader -> NotificationQueue -> consumer drain across a real spawn ->
-- SIGKILL -> respawn cycle. Without this end-to-end coverage, a
-- regression that wired the reader-side gate against the wrong
-- generation source (or dropped the dispatch-side check entirely) would
-- still pass every unit test.
--
-- Shape:
-- 1. Spawn Service A; walk to BootReady. Capture initial generation.
-- 2. Drain queue of whatever boot.progress notifications A emitted
--    (so any post-respawn drain only sees post-respawn notifications,
--    or stale ones that escaped the gate, which we assert against).
-- 3. SIGKILL the child. Wait for respawn ChildSpawned + BootReady
--    (skipping HealthChanged pulses).
-- 4. Capture live generation; assert it has bumped past initial.
-- 5. Drain queue for 500ms post-respawn.
-- 6. For every drained notification with a service_generation field,
--    assert it equals the live generation. Stale-tagged notifications
--    must have been blocked by the reader-side or dispatch-side gate.

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local dir = harness.data_dir("stale_notifications_after_respawn")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")

local initial_gen = client:current_generation()
harness.assert_eq(initial_gen, 1, "first incarnation should be generation 1")

local initial_pid = client:child_pid()
harness.assert(initial_pid ~= nil, "initial child has no pid")

-- Drain whatever Service A queued before SIGKILL so any post-respawn
-- read can only see post-respawn notifications (or stale ones that
-- escaped the gate, which we assert against below).
local queue = client:notifications()
local _ = queue:drain_for(0.2)

harness.kill(initial_pid, "SIGKILL")

-- Respawn ChildSpawned + BootReady (skipping HealthChanged pulses).
local respawn_first = next_lifecycle(events, 15)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
local respawn_second = next_lifecycle(events, 15)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")

local live_gen = client:current_generation()
harness.assert(
    live_gen > initial_gen,
    "respawn must bump current_generation; was " .. tostring(initial_gen)
        .. ", still " .. tostring(live_gen)
)

-- Drain everything the queue contains for 500ms post-respawn and assert
-- no notification carries a stale generation. Allow up to 500ms to catch
-- in-flight notifications from either incarnation.
local drained = queue:drain_for(0.5)
for i, notification in ipairs(drained) do
    if notification.service_generation ~= nil then
        harness.assert_eq(
            notification.service_generation,
            live_gen,
            "drained notification #" .. tostring(i)
                .. " (" .. tostring(notification.type) .. ") carried stale generation "
                .. tostring(notification.service_generation)
                .. " (live=" .. tostring(live_gen) .. ")"
        )
    end
end

local _ = client:shutdown()
