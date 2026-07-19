-- description: B8 Google otherContacts writes seen_addresses without replacing local observations
-- expected: pass
-- fixture: google-other-contacts.toml
-- protocol: people
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- bifrost-google resolves the account from the bearer, so mint a token into
-- saehrimnir's token store (keyed to the fixture's account-1) before seeding.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-google-other", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_google_other"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-other@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-other-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(seed_err == nil, "seed Google account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "Google contact pull")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, seen_address_limit = 50, contact_limit = 50 })
harness.assert(state_err == nil, "seen-address snapshot")
harness.assert(#state.seen_addresses > 0, "otherContacts did not create seen address")
for _, row in ipairs(state.seen_addresses) do
    harness.assert_eq(row.source, "google_other", "otherContacts source")
    harness.assert(not row.local_observed, "other contact must not overwrite local observation")
end

harness.write_summary({ correct = 1, seen_address_count = state.seen_address_count, contact_count = state.contact_count })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
