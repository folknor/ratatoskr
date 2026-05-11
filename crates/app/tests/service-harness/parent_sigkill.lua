-- description: kernel PR_SET_PDEATHSIG terminates Service when its parent is SIGKILLed
-- ceiling: 60s

-- The Service is spawned with `PR_SET_PDEATHSIG = SIGTERM` set on the
-- child via `pre_exec`. When the parent process (parent_death_helper)
-- is SIGKILLed, the kernel delivers SIGTERM to the Service, which then
-- exits within ~2s. This is the kernel-side line of defense for orphan
-- Services if the App crashes hard; cross-platform fallback paths
-- (kill_on_drop, JobObject KILL_ON_JOB_CLOSE) cover the case where
-- the App exits cleanly.
--
-- Linux-only: parent_death_helper's `main` is `#[cfg(target_os =
-- "linux")]`. On other platforms the helper bails with exit code 1 and
-- harness.spawn_parent_death_helper would fail to read the pid.

local dir = harness.data_dir("parent_sigkill")
local info = harness.spawn_parent_death_helper(dir)
harness.assert(info.helper_pid ~= nil, "no helper pid")
harness.assert(info.service_pid ~= nil, "no service pid")
harness.assert(harness.pid_is_alive(info.service_pid), "Service not alive before kill")

harness.kill(info.helper_pid, "SIGKILL")

local deadline = harness.now_ms() + 3000
while harness.now_ms() < deadline do
    if not harness.pid_is_alive(info.service_pid) then
        return
    end
    harness.sleep(50)
end

harness.assert(false,
    "Service pid " .. tostring(info.service_pid) .. " still alive after parent SIGKILL"
)
