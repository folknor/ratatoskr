-- description: spawn_with_events surfaces Terminal { AnotherInstanceRunning } on contention
-- ceiling: 60s

-- Companion to another_instance_running.lua. That script proves the sync
-- spawn flow classifies AnotherInstanceRunning via the returned err
-- shape. This one proves the event-driven spawn_with_events path
-- surfaces the same classification through a Terminal event - which is
-- the angle the App's runtime consumes (the App reacts to SpawnEvent
-- streams from ServiceClient::spawn_with_events_for_*).
--
-- The Service exits with code 71 BEFORE answering the version-check
-- ping, so the AnotherInstanceRunning case can only be classified via
-- the spawn flow's exit-code elevation - the wire-side
-- `ServiceError::BootFailure` path never fires for this case.

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local dir = harness.data_dir("another_instance_terminal")
local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "service A spawn failed")

local events = harness.spawn_with_events(dir)
local terminal = next_lifecycle(events, 15)
harness.assert_eq(terminal.type, "Terminal", "expected Terminal event")

local err = terminal.error
harness.assert(err ~= nil, "Terminal missing error")
harness.assert_eq(err.kind, "BootFailure", "Terminal error kind")
harness.assert_eq(err.classification, "BootFailure", "Terminal classification")
harness.assert_eq(err.boot_code, "AnotherInstanceRunning", "Terminal boot code")
harness.assert_eq(err.boot_code_num, 71, "Terminal boot code num (OS exit code)")

local ok, shutdown_err = client_a:shutdown()
harness.assert(ok, "service A shutdown failed")
harness.assert(shutdown_err == nil, "service A shutdown returned error")
