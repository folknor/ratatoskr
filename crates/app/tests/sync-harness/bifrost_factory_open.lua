-- description: Bifrost account factory attempts a JMAP fixture account open
-- expected: pass
-- fixture: jmap-oauth.toml
-- protocol: jmap
-- ceiling: 120s

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local token_url = harness.join_url(jmap_endpoint, "oauth/token")
local session_url = jmap_endpoint

local dir = harness.data_dir("sync_bifrost_factory_open")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local good, good_err = client:request("TestSeedAccount", {
    email = "bifrost-factory-open@example.test",
    display_name = "Bifrost Factory Open",
    account_name = "Bifrost Factory Open",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = "pre-open-access-token",
    refresh_token = "pre-open-refresh-token",
    token_expires_at = 2000000000,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-harness",
    oauth_token_url = token_url,
    jmap_url = session_url,
})
harness.assert(good_err == nil, "TestSeedAccount good failed")

local exchanged, exchange_err = client:request("oauth.exchange_code", {
    provider_id = "oidc:saehrimnir",
    token_url = token_url,
    scopes = { "openid", "email", "profile" },
    user_info_url = harness.join_url(jmap_endpoint, "oauth/userinfo"),
    use_pkce = false,
    client_id = "ratatoskr-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
    code = "harness-auth-code-bifrost-open",
    reauth_account_id = good.account_id,
})
harness.assert(exchange_err == nil, "oauth.exchange_code for good account failed")
harness.assert_eq(exchanged.email, "test@example.com", "mock userinfo email")

local opened, open_err = client:request("TestBifrostFactoryOpen", {
    account_id = good.account_id,
})
harness.assert(open_err == nil, "TestBifrostFactoryOpen good failed")
harness.assert(
    opened.opened or opened.failure_kind == "NotImplemented",
    "bifrost factory neither opened nor hit the known mock Thread/get gap"
)
if opened.opened then
    harness.assert(opened.capability_debug ~= nil, "open ack missing capabilities")
else
    -- `provider_message` carries the wire-safe message_key; the raw JMAP
    -- cause-chain detail (e.g. "unknownMethod") lives on the test-only
    -- diagnostic_debug field so the safe-key convention is not conflated
    -- with internal diagnostics.
    harness.assert(
        string.find(opened.diagnostic_debug, "unknownMethod", 1, true) ~= nil,
        "expected saehrimnir JMAP method gap"
    )
end

-- Negative case: a misconfigured bearer JMAP account (non-Fastmail OAuth
-- provider with no stored token endpoint) is rejected at construction.
-- saehrimnir's mock token endpoint accepts any refresh token, so an
-- auth-lost open failure is not reproducible against the mock; the
-- deterministic Permanent path through the bifrost layer is the
-- BifrostBuildError::MissingEndpoint rejection, surfaced through the same
-- ack and classified Permanent.
local bad, bad_seed_err = client:request("TestSeedAccount", {
    email = "bifrost-factory-bad@example.test",
    display_name = "Bifrost Factory Bad",
    account_name = "Bifrost Factory Bad",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = "pre-open-access-token",
    refresh_token = "pre-open-refresh-token",
    token_expires_at = 2000000000,
    oauth_provider = "generic-not-fastmail",
    oauth_client_id = "ratatoskr-harness",
    jmap_url = session_url,
})
harness.assert(bad_seed_err == nil, "TestSeedAccount bad failed")

local bad_open, bad_open_err = client:request("TestBifrostFactoryOpen", {
    account_id = bad.account_id,
})
harness.assert(bad_open_err == nil, "TestBifrostFactoryOpen bad request failed")
harness.assert(not bad_open.opened, "misconfigured account must not open")
harness.assert_eq(
    bad_open.failure_kind,
    "Permanent",
    "missing JMAP token endpoint maps to Permanent"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
