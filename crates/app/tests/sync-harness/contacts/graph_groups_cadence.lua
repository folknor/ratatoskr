-- description: Graph directory groups run from the production resident aux cadence
-- expected: pass
-- fixture: graph-groups.lua
-- protocol: graph
-- ceiling: 120s

local client, spawn_err = harness.spawn(harness.data_dir("graph_groups_cadence"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot ready")
local account, seed_err = client:request("TestSeedAccount", {
    email = "owner@example.test", provider = "graph",
})
harness.assert(seed_err == nil, "seed graph account")

-- Do not call TestGroupPull here: this is the sole proof that run_aux_pass
-- drives cycle zero. A real sync attaches the resident account; the resident
-- initial delay is five seconds, so poll with a generous bound for slow CI
-- rather than relying on a fixed sleep.
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil, "initial sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "initial sync result")

local state
for _ = 1, 24 do
    harness.sleep(1000)
    local read, read_err = client:request("TestQueryDbState", {
        account_id = account.account_id, contact_group_limit = 20,
    })
    harness.assert(read_err == nil, "read group state")
    state = read
    if #state.contact_groups == 3 then break end
end
harness.assert(state ~= nil and #state.contact_groups == 3, "cycle-zero group pull did not land")

local group_key = "contact_pull_cycle:" .. account.account_id .. ":directory_groups"
local contact_key = "contact_pull_cycle:" .. account.account_id .. ":graph"
local group_cycle, contact_cycle
for _, setting in ipairs(state.settings) do
    if setting.key == group_key then group_cycle = tonumber(setting.value) end
    if setting.key == contact_key then contact_cycle = tonumber(setting.value) end
end
harness.assert(group_cycle ~= nil and group_cycle >= 1, "directory group cadence counter missing")
harness.assert(contact_cycle ~= nil and contact_cycle >= 1, "contact cadence counter missing")
harness.assert(group_key ~= contact_key, "group and contact cadence keys collided")

harness.write_summary({ correct = 1, group_count = #state.contact_groups })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown")
