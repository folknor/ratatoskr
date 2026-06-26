-- description: Production Gmail kick survives forced lag via resident full-reconcile re-push
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-lag-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local gmail_endpoint = harness.env("RATATOSKR_TEST_GMAIL_ENDPOINT")
harness.assert(gmail_endpoint ~= nil, "RATATOSKR_TEST_GMAIL_ENDPOINT missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_production_lag_backoff")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-lag-backoff@example.test",
    display_name = "Sync Gmail Lag Backoff",
    account_name = "Sync Gmail Lag Backoff",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-lag-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local armed, arm_err = client:request("test.bifrost_arm_hook", {
    account_id = account.account_id,
    hook = { kind = "force_lag" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "force_lag hook was not armed")

harness.clear_mock_requests(admin_endpoint)
local started = harness.now_ms()
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
local elapsed = harness.now_ms() - started
harness.assert(sync_err == nil, "start_sync failed")
harness.assert(elapsed < 30000, "lagged production kick did not terminate within bounded window")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "all messages persist after lag recovery")
harness.assert(state.thread_count >= 1, "thread count")

harness.write_summary({
    correct = 1,
    elapsed_ms = elapsed,
    message_count = state.message_count,
    thread_count = state.thread_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
