-- description: B8 Graph fan-out preserves one row and claim per email
-- expected: pass
-- fixture: graph-contacts-small.toml
-- protocol: graph
-- ceiling: 120s

local client, spawn_err = harness.spawn(harness.data_dir("b8_graph_fanout"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", { email = "b8-fanout@example.test", provider = "graph" })
harness.assert(seed_err == nil, "seed Graph account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "Graph pull")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 50, contact_claim_limit = 50 })
harness.assert(state_err == nil, "Graph snapshot")
local rows, claims = 0, 0
for _, row in ipairs(state.contacts) do
    if row.server_id == "contact-001" then
        rows = rows + 1
        harness.assert(row.phone ~= nil and row.company ~= nil and row.notes ~= nil, "enriched fanout values")
    end
end
for _, claim in ipairs(state.contact_claims) do
    if claim.server_id == "contact-001" then claims = claims + 1 end
end
harness.assert_eq(rows, 2, "Graph contact fan-out rows")
harness.assert_eq(claims, 2, "Graph contact fan-out claims")

harness.write_summary({ correct = 1, contact_count = state.contact_count, claim_count = state.contact_claim_count })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
