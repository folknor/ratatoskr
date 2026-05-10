-- description: JMAP OAuth sync reports a failed result for a revoked bearer token
-- expected: pass
-- fixture: jmap-oauth.toml
-- protocol: jmap
-- ceiling: 120s

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

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
    access_token = "revoked-access-token",
    refresh_token = "revoked-refresh-token",
    token_expires_at = 2000000000,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-harness",
    oauth_token_url = harness.join_url(jmap_endpoint, "oauth/token"),
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local result, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_sync transport failed")
harness.assert(result ~= nil, "start_sync returned nil result")
harness.assert_eq(result.result, "failed", "revoked token sync result")
harness.assert(result.error ~= nil, "revoked token failure missing error")
harness.assert(
    string.find(result.error, "401", 1, true) ~= nil,
    "revoked token failure did not include 401: " .. result.error
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
