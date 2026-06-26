-- description: Graph steady-state sync uses delta endpoints without duplicating mail
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: graph
-- ceiling: 120s

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

local dir = harness.data_dir("sync_graph_steady_state_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-graph-delta@example.test",
    display_name = "Sync Graph Delta",
    account_name = "Sync Graph Delta",
    provider = "graph",
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

harness.clear_mock_requests(graph_endpoint)

harness.marker("SYNC_START")
local second, second_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(second_err == nil, "delta start_sync failed")
harness.assert_eq(second.result, "completed", second.error or "delta sync result")

local requests = harness.mock_requests(graph_endpoint)
local folder_requests =
    harness.request_count(requests, "graph", "GET /v1.0/me/mailFolders")
local message_delta_requests =
    harness.request_count_prefix(requests, "graph", "GET /v1.0/me/mailFolders/")
local master_category_requests =
    harness.request_count(requests, "graph", "GET /v1.0/me/outlook/masterCategories")

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
    graph_folder_requests = folder_requests,
    graph_mail_requests = message_delta_requests,
    graph_master_category_requests = master_category_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
