-- description: B8 GAL uses Bifrost directory_search and leaves unsupported accounts empty
-- expected: pass
-- fixture: contacts-directory.toml
-- protocol: graph
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

-- The Google leg resolves its account from the bearer, so mint a token into
-- saehrimnir's token store (keyed to the fixture's account-1). Graph/JMAP legs
-- do not need this.
local token_url = harness.join_url(admin, "oauth/token")
local minted = harness.http_json({ method = "POST", url = token_url, body = {
    grant_type = "authorization_code", account_id = "account-1",
    code = "harness-contacts-gal-google", client_id = "ratatoskr-contacts-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
} })
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")

local client, spawn_err = harness.spawn(harness.data_dir("b8_gal_directory"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")

local graph, graph_err = client:request("TestSeedAccount", { email = "b8-gal-graph@example.test", provider = "graph" })
harness.assert(graph_err == nil, "seed Graph account")
local google, google_err = client:request("TestSeedAccount", {
    email = "b8-gal-google@example.test", provider = "gmail_api",
    access_token = minted.access_token, refresh_token = "b8-gal-google-refresh-unused",
    token_expires_at = 2000000000, oauth_provider = "google",
    oauth_client_id = "ratatoskr-contacts-harness", oauth_token_url = token_url,
})
harness.assert(google_err == nil, "seed Google account")
local jmap, jmap_err = client:request("TestSeedAccount", { email = "b8-gal-jmap@example.test", provider = "jmap" })
harness.assert(jmap_err == nil, "seed JMAP account")

local _, kick_err = client:request("TestGalKick")
harness.assert(kick_err == nil, "GAL kick")

local gs, gs_err = client:request("TestQueryDbState", { account_id = graph.account_id, gal_cache_limit = 50 })
harness.assert(gs_err == nil, "Graph GAL snapshot")
local os, os_err = client:request("TestQueryDbState", { account_id = google.account_id, gal_cache_limit = 50 })
harness.assert(os_err == nil, "Google GAL snapshot")
local js, js_err = client:request("TestQueryDbState", { account_id = jmap.account_id, gal_cache_limit = 50 })
harness.assert(js_err == nil, "JMAP GAL snapshot")

harness.assert(#gs.gal_cache > 0, "Graph directory missing")
harness.assert(#os.gal_cache > 0, "Google directory missing")
harness.assert_eq(#js.gal_cache, 0, "unsupported JMAP must not cache GAL")
for _, row in ipairs(gs.gal_cache) do
    harness.assert(row.email ~= nil and row.display_name ~= nil, "Graph GAL golden identity")
end

harness.write_summary({ correct = 1, graph_gal_count = #gs.gal_cache, google_gal_count = #os.gal_cache, jmap_gal_count = #js.gal_cache })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
