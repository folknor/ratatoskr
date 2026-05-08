-- description: health.ping responds while boot.ready is parked on a long boot
-- ceiling: 60s

local dir = harness.data_dir("m2_5_health_during_long_migration")
local client, err = harness.spawn(dir, { "--test-boot-delay-ms=1500" })
harness.assert(err == nil, "spawn failed")

local ready_req = client:request_async("BootReady")
harness.sleep(200)

local started = harness.now_ms()
local ping, ping_err = client:request("HealthPing")
local ping_elapsed = harness.now_ms() - started
harness.assert(ping_err == nil, "health ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")
harness.assert(
    ping_elapsed < 1000,
    "health ping waited behind boot.ready; elapsed_ms=" .. ping_elapsed
)

local ready, ready_err = ready_req:await(10)
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
