-- description: JMAP OAuth re-auth persists tokens before OAuth-enforced sync
-- expected: pass
-- fixture: jmap-oauth.toml
-- protocol: jmap
-- ceiling: 120s

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local token_url = harness.join_url(jmap_endpoint, "oauth/token")

local dir = harness.data_dir("sync_jmap_oauth_recovery")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local future_expiry = 2000000000
local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-oauth-recovery@example.test",
    display_name = "Sync JMAP OAuth Recovery",
    account_name = "Sync JMAP OAuth Recovery",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = "pre-reauth-access-token",
    refresh_token = "pre-reauth-refresh-token",
    token_expires_at = future_expiry,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")
local account_id = account.account_id

local ack, oauth_err = client:request("oauth.exchange_code", {
    provider_id = "oidc:saehrimnir",
    token_url = token_url,
    scopes = { "openid", "email", "profile" },
    user_info_url = harness.join_url(jmap_endpoint, "oauth/userinfo"),
    use_pkce = false,
    client_id = "ratatoskr-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
    code = "harness-auth-code-recovery",
    reauth_account_id = account_id,
})
harness.assert(oauth_err == nil, "oauth.exchange_code failed")
harness.assert_eq(ack.email, "test@example.com", "mock userinfo email")
harness.assert(ack.access_token == nil, "reauth ack exposed access token")
harness.assert(ack.refresh_token == nil, "reauth ack exposed refresh token")
harness.assert(ack.token_expires_at == nil, "reauth ack exposed token expiry")

local recovered, recovered_err = client:start_sync({
    account_id = account_id,
}, 30)
harness.assert(recovered_err == nil, "recovered start_sync failed")
harness.assert_eq(recovered.result, "completed", recovered.error or "recovered sync result")

local final_state, final_state_err = client:request("TestQueryDbState", {
    account_id = account_id,
    message_limit = 10,
})
harness.assert(final_state_err == nil, "TestQueryDbState after recovery failed")
harness.assert(final_state.account_count >= 1, "account missing after recovery")
harness.assert_eq(final_state.message_count, 1, "message count after recovery")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
