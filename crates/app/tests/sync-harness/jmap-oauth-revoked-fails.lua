-- description: JMAP OAuth sync reports a failed result for a revoked bearer token
-- expected: pass
-- fixture: jmap-oauth.toml
-- protocol: jmap
-- ceiling: 120s

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local token_url = harness.join_url(jmap_endpoint, "oauth/token")

-- Staging note: fabricated token strings do NOT stage revocation. The mock
-- deliberately treats an unknown-but-not-revoked refresh token as valid
-- (fallback-to-primary), and bifrost recovers from the initial 401 by
-- refreshing - so seeding made-up strings yields a COMPLETED sync. The only
-- unrecoverable path is minting a real pair and explicitly invalidating it:
-- the access token dies AND its paired refresh lands in the revoked set, so
-- the refresh grant returns 400 invalid_grant instead of minting a new token.
local minted = harness.http_json({
    method = "POST",
    url = token_url,
    body = {
        grant_type = "authorization_code",
        code = "harness-auth-code-revoked",
        client_id = "ratatoskr-harness",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(minted.access_token ~= nil, "/oauth/token did not return access_token")
harness.assert(minted.refresh_token ~= nil, "/oauth/token did not return refresh_token")

-- 204 No Content on success, so use the status-carrying helper rather than
-- http_json (which maps an empty body to nil).
local invalidated = harness.http({
    method = "POST",
    url = harness.join_url(jmap_endpoint, "test/oauth/invalidate"),
    content_type = "application/json",
    body = string.format('{"token": %q}', minted.access_token),
})
harness.assert_eq(invalidated.status, 204, "/test/oauth/invalidate status")

local dir = harness.data_dir("sync_jmap_oauth_revoked_fails")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-oauth-revoked@example.test",
    display_name = "Sync JMAP OAuth Revoked",
    account_name = "Sync JMAP OAuth Revoked",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = minted.access_token,
    refresh_token = minted.refresh_token,
    token_expires_at = 2000000000,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local result, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_sync transport failed")
harness.assert(result ~= nil, "start_sync returned nil result")
harness.assert_eq(result.result, "failed", "revoked token sync result")
harness.assert(result.error ~= nil, "revoked token failure missing error")
-- The stable contract is bifrost's error CLASSIFICATION, not a raw HTTP
-- status: the initial 401 is recovered into a refresh attempt, and it is the
-- refresh grant's invalid_grant rejection that surfaces - mapped to
-- Authentication(ReauthorizationRequired) / AuthLost.
harness.assert(
    string.find(result.error, "ReauthorizationRequired", 1, true) ~= nil,
    "revoked token failure was not classified ReauthorizationRequired: " .. result.error
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
