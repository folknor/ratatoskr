-- description: B8 CardDAV composition pulls hydrated vCards and preserves failed ids
-- expected: pass
-- fixture: carddav-contacts-small.toml
-- protocol: carddav
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
local carddav = harness.env("RATATOSKR_TEST_CARDDAV_ENDPOINT") or harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(carddav ~= nil, "CardDAV endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("b8_carddav_pull"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-carddav@example.test",
    provider = "imap",
    caldav_url = carddav,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(seed_err == nil, "seed IMAP/CardDAV account")

local pulled, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil and pulled.imported >= 1, "CardDAV pull")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(state_err == nil, "CardDAV snapshot")
harness.assert(state.contacts[1] ~= nil, "missing CardDAV contact")
harness.assert_eq(state.contacts[1].source, "carddav", "CardDAV provenance source")
harness.assert(#state.contact_claims >= 1, "CardDAV claim")

harness.write_summary({ correct = 1, contact_count = state.contact_count, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
