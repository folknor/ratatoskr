-- description: externally killed Service is detected and recovered
-- ceiling: 90s

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local dir = harness.data_dir("m6_heartbeat_detects_killed_service")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")
harness.assert(boot.response.ready, "initial boot not ready")

local killed_pid = client:child_pid()
harness.assert(killed_pid ~= nil, "pid missing")
harness.kill(killed_pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 20)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "client changed across respawn")

local respawn_second = next_lifecycle(events, 20)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")
harness.assert(respawn_second.response.ready, "respawn boot not ready")

local new_pid = client:child_pid()
harness.assert(new_pid ~= nil, "respawn pid missing")
harness.assert(new_pid ~= killed_pid, "pid did not change")

local ok, err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(err == nil, "shutdown returned error")
