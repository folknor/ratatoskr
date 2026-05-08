-- description: SIGKILL triggers respawn and the replacement answers ping
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

local dir = harness.data_dir("respawn_after_sigkill")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client
local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")

local initial_pid = client:child_pid()
harness.assert(initial_pid ~= nil, "initial pid missing")
harness.kill(initial_pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 20)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "respawn changed client Arc")

local respawn_second = next_lifecycle(events, 20)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")
harness.assert(respawn_second.response.ready, "respawn response not ready")

local respawned_pid = client:child_pid()
harness.assert(respawned_pid ~= nil, "respawn pid missing")
harness.assert(initial_pid ~= respawned_pid, "pid did not change after respawn")

local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "post-respawn ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")
harness.assert_eq(ping.pid, respawned_pid, "ping pid")

local ok, err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(err == nil, "shutdown returned error")
