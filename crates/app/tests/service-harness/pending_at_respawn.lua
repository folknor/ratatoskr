-- description: pending request fails on SIGKILL, then respawned Service answers
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

local dir = harness.data_dir("pending_at_respawn")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client
local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")

local initial_pid = client:child_pid()
harness.assert(initial_pid ~= nil, "initial pid missing")

local pending = client:request_async("TestSlow", { millis = 60000 })
harness.sleep(200)
harness.kill(initial_pid, "SIGKILL")

local slow_result, slow_err = pending:await(5)
harness.assert(slow_result == nil, "slow request unexpectedly succeeded")
harness.assert(slow_err ~= nil, "slow request missing error")
harness.assert_eq(slow_err.kind, "ServiceCrashed", "slow request error")

local respawn_first = next_lifecycle(events, 20)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
local respawn_second = next_lifecycle(events, 20)
harness.assert_eq(respawn_second.type, "BootReady", "respawn BootReady")
harness.assert(respawn_second.response.ready, "respawn response not ready")

local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "post-respawn ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")

local respawned_pid = client:child_pid()
harness.assert(respawned_pid ~= nil, "respawn pid missing")
harness.assert(initial_pid ~= respawned_pid, "pid did not change")

local ok, err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(err == nil, "shutdown returned error")
