-- description: health.ping and boot.ready can complete concurrently
-- ceiling: 60s

local dir = harness.data_dir("m2_5_concurrent_ping_during_boot")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready_req = client:request_async("BootReady")
local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "health ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")

local ready, ready_err = ready_req:await(15)
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
