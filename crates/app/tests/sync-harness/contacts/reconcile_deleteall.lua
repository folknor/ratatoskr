-- description: B8 snapshot reconcile honors a clean delete-all
-- expected: pass
-- fixture: contacts-carddav-reconcile.toml
-- protocol: carddav
-- ceiling: 120s

local dav = harness.env("RATATOSKR_TEST_CARDDAV_ENDPOINT") or harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(dav ~= nil, "CardDAV endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("b8_reconcile_deleteall"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-reconcile@example.test",
    provider = "imap",
    caldav_url = dav,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(seed_err == nil, "seed IMAP/CardDAV account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "initial CardDAV pull")
local before, before_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(before_err == nil, "initial snapshot")
harness.assert(#before.contact_claims > 0, "initial claims")

local empty = harness.http_json({ method = "POST", url = harness.join_url(admin, "test/fixture/step"), body = { expect = "delete_all" } })
harness.assert(empty.ok, "clean empty fixture step")

local _, second_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(second_err == nil, "delete-all reconcile pull")
local deleted, deleted_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(deleted_err == nil, "post-delete snapshot")
harness.assert_eq(#deleted.contact_claims, 0, "clean empty retires claims")

harness.write_summary({ correct = 1, contact_count = deleted.contact_count })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
