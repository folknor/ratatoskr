-- description: Drop escalates to SIGKILL when the Service ignores stdin EOF
-- ceiling: 60s

-- The drop_terminates_child.lua harness script verifies the happy path
-- where the Service exits cleanly when stdin closes. This script verifies
-- the kill-escalation path that is the only line of defense when the
-- happy path doesn't fire. Without it, a regression that removed the
-- start_kill step from Drop's escalation chain would not be caught.
--
-- --test-hang-on-stdin-eof tells the Service to ignore stdin EOF and
-- park indefinitely instead of exiting cleanly. Drop must SIGKILL it.
--
-- Acceptance:
-- - Child is dead within ~3s of client:drop().
-- - Elapsed time is at least ~800ms, proving the kill-escalation path
--   actually fired (the Drop budget is 200ms abort + 1s exit_deadline +
--   start_kill + 500ms poll = ~1.7s; if the child died inside ~200ms
--   that means the happy path took it, which would defeat the test).

local dir = harness.data_dir("drop_kills_hung_service")
local client, err = harness.spawn(dir, { "--test-hang-on-stdin-eof" })
harness.assert(err == nil, "spawn failed")

local pid = client:child_pid()
harness.assert(pid ~= nil, "child_pid missing")
harness.assert(harness.pid_is_alive(pid), "Service should be running before drop")

local started = harness.now_ms()
client:drop()

local deadline = started + 3000
while harness.now_ms() < deadline do
    if not harness.pid_is_alive(pid) then
        local elapsed = harness.now_ms() - started
        harness.assert(
            elapsed >= 800,
            "Drop returned in " .. tostring(elapsed) .. "ms; expected at least ~1s "
                .. "waiting for the hung child before SIGKILL escalates"
        )
        return
    end
    harness.sleep(50)
end

harness.assert(false,
    "wedged Service pid " .. tostring(pid) .. " still alive after Drop deadline; "
        .. "SIGKILL escalation did not fire"
)
