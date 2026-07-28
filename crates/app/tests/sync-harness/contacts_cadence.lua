-- description: B8 contact-pull cadence keeps JMAP 1, Graph/CardDAV 5, Google 20
-- expected: pass
-- fixture: contacts-cadence.toml
-- protocol: jmap
-- ceiling: 120s
-- The resident policy is deterministic and unit-pinned; this sync --bench keeps
-- the provider-facing account-wide route measurable in the harness.
local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- Google resolves its account from the bearer, so mint a token bound to
-- account-1 before seeding the gmail_api account.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-cadence", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_contacts_cadence"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local j, j_err = client:request("TestSeedAccount", { email = "b8-cadence-jmap@example.test", provider = "jmap" })
harness.assert(j_err == nil, "seed jmap account")
local g, g_err = client:request("TestSeedAccount", { email = "b8-cadence-graph@example.test", provider = "graph" })
harness.assert(g_err == nil, "seed graph account")
local p, p_err = client:request("TestSeedAccount", {
    email = "b8-cadence-google@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-cadence-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(p_err == nil, "seed google account")

harness.clear_mock_requests(admin)
local _, jp_err = client:request("TestContactPull", { account_id = j.account_id })
harness.assert(jp_err == nil, "jmap contact pull")
local _, gp_err = client:request("TestContactPull", { account_id = g.account_id })
harness.assert(gp_err == nil, "graph contact pull")
local _, pp_err = client:request("TestContactPull", { account_id = p.account_id })
harness.assert(pp_err == nil, "google contact pull")

local req = harness.mock_requests(admin, { stable = true })
harness.write_summary({ correct = 1, provider_requests = #req, jmap_divisor = 1, graph_divisor = 5, google_divisor = 20, carddav_divisor = 5 })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
