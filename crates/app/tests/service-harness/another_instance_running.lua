-- description: second --service against contended data dir is classified as AnotherInstanceRunning
-- ceiling: 60s

-- Two `--service` instances against the same data dir: the first takes
-- the fs2 file lock at boot; the second hits the contended path and
-- exits with `BootExitCode::AnotherInstanceRunning` (code 71) before
-- responding to the version-check ping. The spawn flow's exit-code
-- elevation classifies this as `BootFailure { AnotherInstanceRunning }`,
-- which is what makes the App surface "Ratatoskr is already running."
-- rather than "Service boot failed: service crashed."
--
-- The covered-from-libtest version asserted only the OS exit code is 71.
-- The Lua port reaches the same property via err.boot_code_num while
-- also asserting the typed boot_code name, which is the angle the App
-- consumes.

local dir = harness.data_dir("another_instance_running")
local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "service A spawn failed")

local client_b, err_b = harness.spawn(dir)
harness.assert(client_b == nil, "service B unexpectedly succeeded")
harness.assert(err_b ~= nil, "service B returned no error")
harness.assert_eq(err_b.kind, "BootFailure", "service B error kind")
harness.assert_eq(err_b.classification, "BootFailure", "service B classification")
harness.assert_eq(err_b.boot_code, "AnotherInstanceRunning", "service B boot code")
harness.assert_eq(err_b.boot_code_num, 71, "service B boot code num (OS exit code)")

local ok, shutdown_err = client_a:shutdown()
harness.assert(ok, "service A shutdown failed")
harness.assert(shutdown_err == nil, "service A shutdown returned error")
