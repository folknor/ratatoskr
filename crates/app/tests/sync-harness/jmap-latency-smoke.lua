-- description: JMAP sync runs with saehrimnir latency controls enabled
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s

local function assert_latency_absent(snapshot, key)
    harness.assert(snapshot[key] == nil, "latency key still present: " .. key)
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local initial_latency = harness.latency(jmap_endpoint)
assert_latency_absent(initial_latency, "global")
assert_latency_absent(initial_latency, "jmap")

local configured = harness.set_latency(jmap_endpoint, {
    global_ms = 20,
    per_protocol = {
        jmap = 5,
    },
})
harness.assert_eq(configured.global, 20, "global latency")
harness.assert_eq(configured.jmap, 5, "jmap latency")

local observed = harness.latency(jmap_endpoint)
harness.assert_eq(observed.global, 20, "observed global latency")
harness.assert_eq(observed.jmap, 5, "observed jmap latency")

local probe_started = harness.now_ms()
local probe = harness.http_json({
    method = "POST",
    url = harness.join_url(jmap_endpoint, "jmap/api"),
    body = "{\"using\":[\"urn:ietf:params:jmap:core\",\"urn:ietf:params:jmap:mail\"],\"methodCalls\":[[\"Mailbox/get\",{\"accountId\":\"account-1\"},\"c0\"]]}",
})
local probe_elapsed = harness.now_ms() - probe_started
harness.assert(probe.methodResponses ~= nil, "latency probe missing JMAP response")
-- Global latency stacks with per-protocol latency; assert at least the
-- global portion so the test stays tolerant of timer granularity.
harness.assert(
    probe_elapsed >= 15,
    "latency probe returned before configured delay"
)

local dir = harness.data_dir("sync_jmap_latency_smoke")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-latency@example.test",
    display_name = "Sync JMAP Latency",
    account_name = "Sync JMAP Latency",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local result, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "latency start_sync failed")
harness.assert_eq(result.result, "completed", result.error or "latency sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "latency sync message count")

local cleared = harness.set_latency(jmap_endpoint, {
    global_ms = 0,
    per_protocol = {
        jmap = 0,
    },
})
assert_latency_absent(cleared, "global")
assert_latency_absent(cleared, "jmap")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
