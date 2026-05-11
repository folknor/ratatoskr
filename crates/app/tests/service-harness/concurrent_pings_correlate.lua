-- description: 100 concurrent HealthPing requests round-trip with correct id correlation
-- ceiling: 60s

-- The Service dispatch loop must correlate every response to the request
-- id it carried, even with many requests in flight. A regression in the
-- pending-map or the id-extraction path would surface as missing IDs
-- (some pings hang) or as IDs landing on the wrong response slot
-- (resolved to the wrong pending future). 100 is enough to saturate
-- typical pending-map sizes; the test asserts the full set 1..=100 is
-- accounted for in the response stream.

local dir = harness.data_dir("concurrent_pings_correlate")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local pending = {}
for i = 1, 100 do
    pending[i] = client:request_async("HealthPing")
end

local seen = {}
for i = 1, 100 do
    local response, ping_err = pending[i]:await(10)
    harness.assert(ping_err == nil, "ping " .. tostring(i) .. " errored")
    harness.assert_eq(response.version, harness.protocol_version, "ping " .. i .. " version")
    seen[i] = true
end

for i = 1, 100 do
    harness.assert(seen[i], "ping " .. tostring(i) .. " did not return")
end

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
