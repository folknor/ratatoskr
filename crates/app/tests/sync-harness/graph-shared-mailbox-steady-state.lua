-- description: Graph shared-mailbox steady-state delta polls one delta per foreign folder
-- expected: pass
-- fixture: shared-graph-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_shared_mailbox_steady_state

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local graph_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(graph_endpoint ~= nil, "saehrimnir admin endpoint missing")

local dir = harness.data_dir("sync_graph_shared_mailbox_steady_state")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test",
    display_name = "Viewer",
    account_name = "Viewer",
    provider = "graph",
    delegate_discovery_enabled = true,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local first, first_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(first_err == nil, "initial start_sync failed")
harness.assert_eq(first.result, "completed", first.error or "initial sync result")

local after_initial, after_initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(after_initial_err == nil, "TestQueryDbState after initial sync failed")
local shared = nil
for _, message in ipairs(after_initial.messages) do
    if message.subject == "Shared Graph copy" then
        shared = message
    end
end
harness.assert(shared ~= nil, "foreign Graph message missing after initial sync")

-- Wait out the resident aux pass (5s after attach) and let the request log
-- go quiet so the window below measures the delta kick and nothing else.
harness.sleep(6000)
local quiesce = #harness.mock_requests(graph_endpoint)
for _ = 1, 20 do
    harness.sleep(500)
    local now = #harness.mock_requests(graph_endpoint)
    if now == quiesce then
        break
    end
    quiesce = now
end

harness.clear_mock_requests(graph_endpoint)

harness.marker("SYNC_START")
local second, second_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(second_err == nil, "delta start_sync failed")
harness.assert_eq(second.result, "completed", second.error or "delta sync result")

local requests = harness.mock_requests(graph_endpoint)
-- One delta per folder, personal and foreign, and nothing else: no
-- per-kick delegate rediscovery, no personal-path duplication (B12
-- namespaced steady-state cost contract).
local personal_delta_requests =
    harness.request_count_prefix(requests, "graph", "GET /v1.0/me/mailFolders/")
local foreign_delta_requests =
    harness.request_count_prefix(requests, "graph", "GET /v1.0/users/")

local after_delta, after_delta_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(after_delta_err == nil, "TestQueryDbState after delta sync failed")
harness.assert_eq(after_delta.message_count, after_initial.message_count, "delta message count")
harness.assert_eq(after_delta.thread_count, after_initial.thread_count, "delta thread count")
local delta_account = account_by_id(after_delta, account.account_id)
harness.assert(delta_account ~= nil, "account missing after delta sync")
harness.assert(delta_account.initial_sync_completed, "delta cleared initial sync flag")

harness.write_summary({
    correct = 1,
    message_count = after_delta.message_count,
    thread_count = after_delta.thread_count,
    provider_requests = #requests,
    graph_personal_mail_requests = personal_delta_requests,
    graph_foreign_mail_requests = foreign_delta_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
