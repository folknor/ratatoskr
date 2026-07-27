-- description: Graph delegate discovery is opt-in and bootstraps the shared mailbox registry
-- expected: pass
-- fixture: shared-graph-small.toml
-- protocol: graph
-- ceiling: 120s
-- @covers: bifrost.b12.graph_delegate_discovery

local function sync(client, account_id, label)
    local result, err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync failed"))
end

local function shared_message(state)
    for _, message in ipairs(state.messages) do
        if message.subject == "Shared Graph copy" then return message end
    end
    return nil
end

local function run(enabled)
    local dir = harness.data_dir(enabled and "graph_delegate_enabled" or "graph_delegate_disabled")
    local client, err = harness.spawn(dir)
    harness.assert(err == nil, "spawn failed")
    local ready, ready_err = client:request("BootReady")
    harness.assert(ready_err == nil and ready.ready, "boot failed")
    local account, account_err = client:request("TestSeedAccount", {
        email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer",
        provider = "graph", delegate_discovery_enabled = enabled,
    })
    harness.assert(account_err == nil, "seed failed")
    sync(client, account.account_id, enabled and "enabled" or "disabled")
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account.account_id, message_limit = 20,
    })
    harness.assert(state_err == nil, "state read failed")
    local message = shared_message(state)
    if enabled then
        harness.assert(message ~= nil, "delegate Autodiscover did not discover the foreign mailbox")
        harness.assert(message.id ~= "alice-message", "delegate message id is not namespaced")
    else
        harness.assert(message == nil, "disabled delegate discovery imported foreign mail")
    end
    local ok, shutdown_err = client:shutdown()
    harness.assert(ok and shutdown_err == nil, "shutdown failed")
end

run(false)
run(true)
