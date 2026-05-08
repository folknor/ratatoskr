-- description: M1 smoke test for app --test-harness
-- ceiling: 60s

local dir = harness.data_dir("smoke")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "health ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
