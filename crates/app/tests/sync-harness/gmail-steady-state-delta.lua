-- description: Gmail steady-state sync uses history without duplicating mail
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-delta-account-1",
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
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_steady_state_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-delta@example.test",
    display_name = "Sync Gmail Delta",
    account_name = "Sync Gmail Delta",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-delta-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(before_err == nil, "TestQueryDbState before sync failed")
local before_account = account_by_id(before, account.account_id)
harness.assert(before_account ~= nil, "account missing before sync")
harness.assert(
    not before_account.initial_sync_completed,
    "initial_sync_completed set before first sync"
)

local first, first_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(first_err == nil, "initial start_sync failed")
harness.assert_eq(first.result, "completed", first.error or "initial sync result")

local after_initial, after_initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_initial_err == nil, "TestQueryDbState after initial sync failed")
harness.assert_eq(after_initial.message_count, 2, "initial message count")
harness.assert(after_initial.thread_count >= 1, "initial thread count")
local synced_account = account_by_id(after_initial, account.account_id)
harness.assert(synced_account ~= nil, "account missing after initial sync")
harness.assert(
    synced_account.initial_sync_completed,
    "initial sync did not mark account completed"
)

harness.clear_mock_requests(admin_endpoint)

harness.marker("SYNC_START")
local second, second_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(second_err == nil, "delta start_sync failed")
harness.assert_eq(second.result, "completed", second.error or "delta sync result")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local profile_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/profile")
local label_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/labels")
local history_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/history")
local message_list_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/messages")
harness.assert(profile_requests >= 1, "delta sync did not fetch profile")
harness.assert(label_requests >= 1, "delta sync did not refresh labels")
harness.assert(history_requests >= 1, "delta sync did not poll history")
harness.assert(
    message_list_requests <= 1,
    "delta sync ran more than the bifrost one-shot backfill message list"
)

local after_delta, after_delta_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_delta_err == nil, "TestQueryDbState after delta sync failed")
harness.assert_eq(after_delta.message_count, after_initial.message_count, "delta message count")
harness.assert_eq(after_delta.thread_count, after_initial.thread_count, "delta thread count")
harness.assert_eq(after_delta.label_count, after_initial.label_count, "delta label count")
local delta_account = account_by_id(after_delta, account.account_id)
harness.assert(delta_account ~= nil, "account missing after delta sync")
harness.assert(delta_account.initial_sync_completed, "delta cleared initial sync flag")

harness.write_summary({
    correct = 1,
    message_count = after_delta.message_count,
    thread_count = after_delta.thread_count,
    label_count = after_delta.label_count,
    provider_requests = #requests,
    gmail_profile_requests = profile_requests,
    gmail_label_requests = label_requests,
    gmail_history_requests = history_requests,
    gmail_message_list_requests = message_list_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
