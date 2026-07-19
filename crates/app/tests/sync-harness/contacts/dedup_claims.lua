-- description: B8 cross-provider claims preserve a shared materialized contact until final retire
-- expected: pass
-- fixture: contacts-dedup.toml
-- protocol: graph
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- The Google leg resolves its account from the bearer, so mint a token into
-- saehrimnir's token store (keyed to the fixture's account-1). The Graph leg
-- does not need this.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-dedup-google", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_dedup_claims"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local google, google_err = client:request("TestSeedAccount", {
    email = "b8-dedup-google@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-dedup-google-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(google_err == nil, "seed Google account")
local graph, graph_err = client:request("TestSeedAccount", { email = "b8-dedup-graph@example.test", provider = "graph" })
harness.assert(graph_err == nil, "seed Graph account")

local _, gp_err = client:request("TestContactPull", { account_id = google.account_id })
harness.assert(gp_err == nil, "Google pull")
local _, xp_err = client:request("TestContactPull", { account_id = graph.account_id })
harness.assert(xp_err == nil, "Graph pull")

local state, state_err = client:request("TestQueryDbState", { contact_limit = 100, contact_claim_limit = 100 })
harness.assert(state_err == nil, "cross-provider snapshot")
local claims = 0
for _, claim in ipairs(state.contact_claims) do if claim.email == "alice@example.com" then claims = claims + 1 end end
harness.assert_eq(claims, 2, "one claim per provider")
local rows = 0
for _, contact in ipairs(state.contacts) do if contact.email == "alice@example.com" then rows = rows + 1 end end
harness.assert_eq(rows, 1, "deduplicated materialization")

harness.write_summary({ correct = 1, contact_count = state.contact_count, claim_count = state.contact_claim_count })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
