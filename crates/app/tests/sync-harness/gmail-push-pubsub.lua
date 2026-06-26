-- description: Gmail Pub/Sub push imports a new message without a second sync kick
-- @covers: architecture.folder_vs_label_semantics_are_explicit
-- expected: pass
-- fixture: jmap-incremental.lua
-- protocol: gmail
-- ceiling: 120s

local function message_by_id(state, id)
    for _, message in ipairs(state.messages) do
        if message.id == id then
            return message
        end
    end
    return nil
end

local function query_state(client, account_id)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, "TestQueryDbState failed")
    return state
end

local function wait_for_message(client, account_id, id, label)
    local deadline = harness.now_ms() + 5000
    while harness.now_ms() < deadline do
        local state = query_state(client, account_id)
        local message = message_by_id(state, id)
        if message ~= nil then
            return message, state
        end
        harness.sleep(100)
    end
    harness.assert(false, label .. " did not arrive through push")
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-push-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local function start_sync(client, account_id, label)
    local result, err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local function apply_step(endpoint, step_id)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "test/fixture/step"),
        body = { expect = step_id },
    })
    harness.assert(response.ok, "fixture step failed")
    harness.assert_eq(response.step, step_id, "fixture step id")
    return response
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local access_token = mint_token(token_url)
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_gmail_push_pubsub")
local client, err = harness.spawn(dir, nil, {
    RATATOSKR_GMAIL_PUBSUB_TOPIC = "projects/saehrimnir/topics/mock",
    RATATOSKR_GMAIL_PUBSUB_SUBSCRIPTION = "projects/saehrimnir/subscriptions/mock",
    RATATOSKR_GMAIL_PUBSUB_MOCK_ENDPOINT = admin_endpoint,
})
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Gmail Push",
    account_name = "Gmail Push",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-push-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

start_sync(client, account.account_id, "initial")
local initial = query_state(client, account.account_id)
harness.assert_eq(initial.message_count, 2, "initial message count")

local subscription_requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(
    harness.request_count(subscription_requests, "gmail", "POST /gmail/v1/users/me/watch") >= 1,
    "Gmail push did not create users.watch"
)
harness.sleep(500)
harness.http_delete(harness.join_url(admin_endpoint, "test/gmail/pubsub/messages"))
harness.clear_mock_requests(admin_endpoint)
apply_step(admin_endpoint, "new")
local early_pubsub = harness.http_get(harness.join_url(admin_endpoint, "test/gmail/pubsub/messages"))
harness.assert(#early_pubsub >= 1, "saehrimnir did not publish Gmail Pub/Sub before wait")
local pushed = wait_for_message(client, account.account_id, "email-003", "email-003")
harness.assert_eq(pushed.subject, "Lunch?", "pushed subject")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/history") >= 1,
    "push reconcile did not call Gmail history"
)
local pubsub = harness.http_get(harness.join_url(admin_endpoint, "test/gmail/pubsub/messages"))
harness.assert(#pubsub >= 1, "saehrimnir did not publish Gmail Pub/Sub")

harness.write_summary({
    correct = 1,
    message_count = query_state(client, account.account_id).message_count,
    provider_requests = #requests,
    pubsub_messages = #pubsub,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
