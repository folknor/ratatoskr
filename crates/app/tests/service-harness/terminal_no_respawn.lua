-- description: Terminal at initial boot must not trigger a respawn loop
-- ceiling: 60s

-- handle_crash sees first_boot_ready is None on a boot-time failure and
-- defers to run_spawn_flow (which already surfaces Terminal). No respawn
-- event should follow. This closes the "missing key produces one
-- Service-per-second forever" concern.

local function expect_terminal(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed before Terminal")
        if event.type == "Terminal" then
            return event.error
        end
    end
end

local dir = harness.data_dir("terminal_no_respawn", false)
local events = harness.spawn_with_events(dir)
local err = expect_terminal(events, 30)

harness.assert(err ~= nil, "Terminal missing error")
harness.assert(
    err.boot_code == "KeyLoadFailure" or err.service_kind == "BootFailure",
    "expected KeyLoadFailure"
)

-- After Terminal: assert no follow-up event arrives in the respawn window
-- (~1s sleep + spawn + boot.ready, conservatively 4s).
harness.expect_quiet(events, 4)
