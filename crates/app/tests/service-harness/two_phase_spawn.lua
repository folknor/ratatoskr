-- description: spawn_with_events emits ChildSpawned before BootReady
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

local dir = harness.data_dir("two_phase_spawn")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "first event")
harness.assert(first.client ~= nil, "ChildSpawned missing client")

local second = next_lifecycle(events, 15)
harness.assert_eq(second.type, "BootReady", "second event")
harness.assert(second.response.ready, "BootReady response not ready")
harness.assert_eq(second.response.schema_version, 100, "schema version")
harness.assert_eq(second.response.migrations_applied, 1, "migration count")

local ok, err = first.client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(err == nil, "shutdown returned error")
