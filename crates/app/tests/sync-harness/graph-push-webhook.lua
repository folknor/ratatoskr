-- description: Graph webhook push imports a new message without a second sync kick
-- @covers: architecture.folder_vs_label_semantics_are_explicit
-- expected: pass
-- fixture: jmap-incremental.lua
-- protocol: graph
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
harness.clear_mock_requests(admin_endpoint)

local loopback = harness.free_tcp_addr()
local notification_url = "http://" .. loopback .. "/graph/push"
local dir = harness.data_dir("sync_graph_push_webhook")
local client, err = harness.spawn(dir, nil, {
    RATATOSKR_GRAPH_PUSH_LOOPBACK = loopback,
    RATATOSKR_GRAPH_PUSH_NOTIFICATION_URL = notification_url,
})
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "test@example.com",
    display_name = "Graph Push",
    account_name = "Graph Push",
    provider = "graph",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

start_sync(client, account.account_id, "initial")
local initial = query_state(client, account.account_id)
harness.assert_eq(initial.message_count, 2, "initial message count")

local subscription_requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(
    harness.request_count(subscription_requests, "graph", "POST /v1.0/subscriptions") >= 1,
    "Graph push did not create a subscription"
)
harness.sleep(500)
harness.clear_mock_requests(admin_endpoint)
apply_step(admin_endpoint, "new")
local early_graph_log = harness.http_get(harness.join_url(admin_endpoint, "test/push/graph"))
harness.assert(#early_graph_log >= 1, "saehrimnir did not emit a Graph webhook before wait")
local pushed = wait_for_message(client, account.account_id, "email-003", "email-003")
harness.assert_eq(pushed.subject, "Lunch?", "pushed subject")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(
    harness.request_count_prefix(requests, "graph", "GET /v1.0/me/mailFolders/") >= 1,
    "push reconcile did not call Graph mail delta"
)
local graph_log = harness.http_get(harness.join_url(admin_endpoint, "test/push/graph"))
harness.assert(#graph_log >= 1, "saehrimnir did not emit a Graph webhook")

harness.write_summary({
    correct = 1,
    message_count = query_state(client, account.account_id).message_count,
    provider_requests = #requests,
    graph_notifications = #graph_log,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
