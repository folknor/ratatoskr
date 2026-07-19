-- description: account.verify opens a Gmail OAuth mailbox without persisting an account
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)
local token = harness.http_json({
    method = "POST",
    url = harness.join_url(admin_endpoint, "oauth/token"),
    body = {
        grant_type = "authorization_code",
        account_id = "account-1",
        code = "account-verify-oauth",
        client_id = "ratatoskr-gmail-harness",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(token.access_token ~= nil, "mock did not mint access token")

local client, err = harness.spawn(harness.data_dir("account_verify_oauth"))
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot.ready failed")

local ack, verify_err = client:request("account.verify", {
    provider = "gmail_api",
    email = "verify@gmail.example.test",
    access_token = token.access_token,
    accept_invalid_certs = false,
})
harness.assert(verify_err == nil, "account.verify transport failed")
harness.assert(ack.ok, ack.message or "OAuth verify failed")

local state, state_err = client:request("TestQueryDbState", {})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.account_count, 0, "verify persisted an account row")
local requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/profile") >= 1, "expected Gmail profile probe")

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
