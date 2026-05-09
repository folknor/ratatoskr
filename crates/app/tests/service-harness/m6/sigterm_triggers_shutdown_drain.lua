-- description: SIGTERM exits the Service through the unrequested drain path
-- ceiling: 30s

local function wait_until_dead(pid, timeout_ms)
    local deadline = harness.now_ms() + timeout_ms
    while harness.now_ms() < deadline do
        if not harness.pid_is_alive(pid) then
            return true
        end
        harness.sleep(50)
    end
    return not harness.pid_is_alive(pid)
end

local dir = harness.data_dir("m6_sigterm_shutdown_drain")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local pid = client:child_pid()
harness.assert(pid ~= nil, "pid missing")
harness.kill(pid, "SIGTERM")

harness.assert(wait_until_dead(pid, 5000), "Service did not exit after SIGTERM")
harness.assert(
    not harness.path_exists(dir .. "/clean_shutdown"),
    "external SIGTERM should not write clean_shutdown sentinel"
)

client:drop()
