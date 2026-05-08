-- description: keyless boot surfaces Terminal KeyLoadFailure
-- ceiling: 60s

local function expect_terminal(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed before Terminal")
        if event.type == "Terminal" then
            return event.error
        end
    end
end

local dir = harness.data_dir("terminal_on_missing_key", false)
local events = harness.spawn_with_events(dir)
local err = expect_terminal(events, 30)

harness.assert(err ~= nil, "Terminal missing error")
harness.assert(
    err.boot_code == "KeyLoadFailure" or err.service_kind == "BootFailure",
    "expected KeyLoadFailure"
)
