-- description: B8 Graph pull preserves fan-out, enriched values, and snapshot reconcile
-- expected: pass
-- fixture: contacts-graph-pull.toml
-- protocol: graph
-- ceiling: 120s

local function by_email(rows, email)
    for _, row in ipairs(rows) do if row.email == email then return row end end
end

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("b8_graph_pull"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", { email = "b8-graph@example.test", provider = "graph" })
harness.assert(seed_err == nil, "seed Graph account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "Graph pull")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(state_err == nil, "Graph snapshot")
local alice = by_email(state.contacts, "alice@example.com")
local alternate = by_email(state.contacts, "alice.anderson@example.org")
harness.assert(alice ~= nil and alternate ~= nil, "Graph fan-out lost alternate email")
harness.assert_eq(alice.server_id, alternate.server_id, "fan-out native identity")
harness.assert_eq(alice.source, "graph", "Graph source")
harness.assert(#state.contact_claims >= 2, "one claim per Graph fan-out row")

local step = harness.http_json({ method = "POST", url = harness.join_url(admin, "test/fixture/step"), body = { expect = "delete" } })
harness.assert(step.ok, "apply delete fixture step")
local _, second_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(second_err == nil, "Graph reconcile pull")

harness.write_summary({ correct = 1, contact_count = state.contact_count, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
