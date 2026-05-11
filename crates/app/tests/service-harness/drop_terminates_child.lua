-- description: dropping ServiceClient terminates the child within 1.5s
-- ceiling: 60s

local dir = harness.data_dir("drop_terminates_child")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local pid = client:child_pid()
harness.assert(pid ~= nil, "child_pid missing")

client:drop()

local deadline = harness.now_ms() + 1500
while harness.now_ms() < deadline do
    if not harness.pid_is_alive(pid) then
        return
    end
    harness.sleep(50)
end

harness.assert(false, "child still alive after Drop deadline")
