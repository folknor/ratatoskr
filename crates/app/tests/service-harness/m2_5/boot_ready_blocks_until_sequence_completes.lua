-- description: boot.ready waits for the boot sequence completion signal
-- ceiling: 60s

local dir = harness.data_dir("m2_5_boot_ready_blocks")
local client, err = harness.spawn(dir, { "--test-boot-delay-ms=1500" })
harness.assert(err == nil, "spawn failed")

local started = harness.now_ms()
local ready, ready_err = client:request("BootReady")
local elapsed = harness.now_ms() - started
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")
harness.assert(
    elapsed >= 400,
    "boot.ready returned before the artificial boot delay; elapsed_ms=" .. elapsed
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
