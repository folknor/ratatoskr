-- description: oauth.exchange_code re-auth persists mock-provider tokens
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 90s

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("m6_oauth_reauth_mock_provider")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-oauth-reauth@example.test",
    display_name = "OAuth Reauth",
    account_name = "OAuth Reauth",
    provider = "graph",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local before_state, before_state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(before_state_err == nil, "TestQueryDbState before reauth failed")
local before = account_by_id(before_state, account.account_id)
harness.assert(before ~= nil, "seeded account missing before reauth")
harness.assert_eq(before.provider, "graph", "provider")
harness.assert_eq(before.auth_method, "oauth2", "auth method")
harness.assert(before.access_token_present, "seed access token missing")
harness.assert(before.refresh_token_present, "seed refresh token missing")
harness.assert(
    not before.access_token_encrypted,
    "seed access token should start as plaintext test data"
)
harness.assert(
    not before.refresh_token_encrypted,
    "seed refresh token should start as plaintext test data"
)

local reauth_started_at = math.floor(harness.now_ms() / 1000)
local ack, oauth_err = client:request("oauth.exchange_code", {
    provider_id = "oidc:saehrimnir",
    token_url = harness.join_url(jmap_endpoint, "oauth/token"),
    scopes = { "openid", "email", "profile" },
    user_info_url = harness.join_url(jmap_endpoint, "oauth/userinfo"),
    use_pkce = false,
    client_id = "ratatoskr-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
    code = "harness-auth-code",
    reauth_account_id = account.account_id,
})
harness.assert(oauth_err == nil, "oauth.exchange_code failed")
harness.assert_eq(ack.email, "test@example.com", "mock userinfo email")
harness.assert_eq(ack.display_name, "test@example.com", "mock userinfo display name")
harness.assert(ack.access_token == nil, "reauth ack exposed access token")
harness.assert(ack.refresh_token == nil, "reauth ack exposed refresh token")
harness.assert(ack.token_expires_at == nil, "reauth ack exposed token expiry")

local after_state, after_state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(after_state_err == nil, "TestQueryDbState after reauth failed")
local after = account_by_id(after_state, account.account_id)
harness.assert(after ~= nil, "seeded account missing after reauth")
harness.assert_eq(after.email, before.email, "email changed")
harness.assert_eq(after.provider, before.provider, "provider changed")
harness.assert_eq(after.auth_method, before.auth_method, "auth method changed")
harness.assert_eq(after.oauth_provider, before.oauth_provider, "oauth provider changed")
harness.assert_eq(after.oauth_client_id, before.oauth_client_id, "oauth client id changed")
harness.assert(after.access_token_present, "access token missing after reauth")
harness.assert(after.refresh_token_present, "refresh token missing after reauth")
harness.assert(after.access_token_encrypted, "access token was not encrypted")
harness.assert(after.refresh_token_encrypted, "refresh token was not encrypted")
harness.assert(
    after.access_token_sha256 ~= before.access_token_sha256,
    "access token hash did not change"
)
harness.assert(
    after.refresh_token_sha256 ~= before.refresh_token_sha256,
    "refresh token hash did not change"
)
harness.assert(after.token_expires_at ~= nil, "token expiry missing after reauth")
harness.assert(
    after.token_expires_at >= reauth_started_at,
    "token expiry predates reauth"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
