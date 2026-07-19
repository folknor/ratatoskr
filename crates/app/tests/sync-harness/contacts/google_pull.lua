-- description: B8 Google People pull maps enriched contacts through the Bifrost surface
-- expected: pass
-- fixture: contacts-google-pull.toml
-- protocol: people
-- ceiling: 120s

local function by_email(rows, email)
    for _, row in ipairs(rows) do if row.email == email then return row end end
end

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- bifrost-google resolves the fixture account from the bearer token, so the
-- account must carry a token minted into saehrimnir's token store (keyed to the
-- fixture's primary account-1); a bare seed leaves every People call unauthed.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-google-pull", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_google_pull"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local account, seed_err = client:request("TestSeedAccount", {
    email = "b8-google@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-google-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(seed_err == nil, "seed Google account")

local _, pull_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(pull_err == nil, "Google contact pull")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_limit = 20, contact_claim_limit = 20 })
harness.assert(state_err == nil, "Google snapshot")
local alice = by_email(state.contacts, "alice@example.com")
harness.assert(alice ~= nil, "missing Google Alice")
harness.assert_eq(alice.source, "google", "Google source")
harness.assert(alice.email2 ~= nil, "packed secondary email missing")

local step = harness.http_json({ method = "POST", url = harness.join_url(admin, "test/fixture/step"), body = { expect = "new" } })
harness.assert(step.ok, "apply add fixture step")
local _, second_err = client:request("TestContactPull", { account_id = account.account_id })
harness.assert(second_err == nil, "Google update pull")

harness.write_summary({ correct = 1, contact_count = state.contact_count, provider_requests = #harness.mock_requests(admin) })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
