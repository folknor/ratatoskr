-- description: boot.ready returns the completed boot sequence response
-- ceiling: 60s

local dir = harness.data_dir("m2_5_boot_ready_completes")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")
harness.assert_eq(ready.schema_version, 100, "schema version")
harness.assert_eq(ready.migrations_applied, 1, "migration count")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
