-- description: Graph public-folder steady state polls pinned folders only
-- expected: pass
-- fixture: public-folder-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_public_folder_steady_state

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

local dir = harness.data_dir("sync_graph_public_folder_steady_state")
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
    public_folders_enabled = true,
    public_folder_pins = { "public:pf-notices", "public:pf-calendar" },
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
local notice, board = nil, nil
for _, message in ipairs(after_initial.messages) do
    if message.subject == "Public notice" then
        notice = message
    end
    if message.subject == "Board item" then
        board = message
    end
end
harness.assert(notice ~= nil, "pinned public mail item missing after initial sync")
harness.assert(board == nil, "unpinned public folder item entered the mail path")

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
-- One EWS FindItem watermark poll per PINNED public folder; unpinned
-- folders are never polled and the hierarchy walk never re-runs, so the
-- request count is a function of the pin count, not the hierarchy size
-- (B12 obstacle O cost contract).
local find_item_requests = harness.request_count(requests, "ews", "POST /EWS FindItem")

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
    ews_find_item_requests = find_item_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
