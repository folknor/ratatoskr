-- description: B8 JMAP ContactCard pull maps enriched fields and reconciles snapshots
-- expected: pass
-- fixture: contacts-jmap-pull.toml
-- protocol: jmap
-- ceiling: 120s

local function by_email(rows, email)
    for _, row in ipairs(rows) do if row.email == email then return row end end
end

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("b8_jmap_pull"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", { email = "b8-jmap@example.test", provider = "jmap" })
harness.assert(seed_err == nil, "seed JMAP account")

local pull, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "initial contact pull")
harness.assert(pull.imported >= 1, "JMAP pull imported no rows")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(state_err == nil, "contact snapshot")
local alice = by_email(state.contacts, "alice@example.com")
harness.assert(alice ~= nil, "missing Alice")
harness.assert_eq(alice.source, "jmap", "JMAP source")
harness.assert(alice.server_id ~= nil, "JMAP native id")
harness.assert(#state.contact_claims >= 1, "JMAP claims")

local stepped = harness.http_json({ method = "POST", url = harness.join_url(admin, "test/fixture/step"), body = { expect = "change" } })
harness.assert(stepped.ok, "apply update fixture step")
local changed, changed_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(changed_err == nil and changed.imported >= 1, "updated contact pull")

harness.write_summary({ correct = 1, contact_count = state.contact_count, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
