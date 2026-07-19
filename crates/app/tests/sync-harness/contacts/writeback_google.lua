-- description: B8 Google contact update/delete use engine passthroughs
-- expected: pass
-- fixture: contacts-google-pull.toml
-- protocol: people
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- bifrost-google resolves the account from the bearer, so mint a token into
-- saehrimnir's token store (keyed to the fixture's account-1) before seeding.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-writeback-google", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_writeback_google"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-w-google@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-w-google-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(seed_err == nil, "seed Google account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "contact pull")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 10 })
harness.assert(state_err == nil, "snapshot")
local contact = state.contacts[1]
harness.assert(contact ~= nil, "contact")

local saved, save_err = client:request("contacts.contact_save_with_writeback", {
    id = contact.id, email = contact.email, display_name = contact.display_name,
    phone = "+1 555 0102", company = "Bifrost", notes = "Google writeback",
    account_id = account.account_id, source = contact.source, server_id = contact.server_id, groups = {},
})
harness.assert(save_err == nil, "save")
harness.assert_eq(saved.writeback.kind, "success", "save outcome")

local deleted, delete_err = client:request("contacts.contact_delete", { id = contact.id })
harness.assert(delete_err == nil, "delete")
harness.assert_eq(deleted.writeback.kind, "success", "delete outcome")

harness.write_summary({ correct = 1, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
