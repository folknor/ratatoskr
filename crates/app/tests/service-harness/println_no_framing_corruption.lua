-- description: println from a handler does not corrupt the JSON-RPC pipe
-- ceiling: 60s

-- Without the Service's stdio-defense (dup the original stdin/stdout to saved
-- fds, redirect the globals to /dev/null), a println from inside a handler
-- would write directly into the JSON-RPC pipe and the next request would fail
-- to parse. With it in place, TestPrintln returns cleanly and a follow-up
-- HealthPing still round-trips.

local dir = harness.data_dir("println_no_framing_corruption")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local _, println_err = client:request("TestPrintln", {
    message = "STDIO-CORRUPTION-CANARY-XYZ",
})
harness.assert(println_err == nil, "TestPrintln failed")

local ping, ping_err = client:request("HealthPing")
harness.assert(ping_err == nil, "post-println ping failed")
harness.assert_eq(ping.version, harness.protocol_version, "protocol version")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
