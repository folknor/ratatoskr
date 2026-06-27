-- description: JMAP mailbox create/rename/move/delete round-trips through engine.container_*
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- B6b container-CRUD gate. The folder CRUD action handlers
-- (actions::folder::{create,rename,move,delete}) moved off ProviderOps onto
-- the bifrost engine's container_* primitives. The harness-only
-- test.container_crud trigger drives each handler; a follow-up resync's
-- containers_list must reflect each step on the server - a real round-trip,
-- not just a local write.

local function folder_by_name(folders, name)
    for _, folder in ipairs(folders) do
        if folder.name == name then
            return folder
        end
    end
    return nil
end

local function resync(client, account_id, label)
    local r, e = client:start_sync({ account_id = account_id }, 30)
    harness.assert(e == nil, label .. " resync failed")
    harness.assert_eq(r.result, "completed", r.error or (label .. " resync result"))
end

local function db_state(client, account_id)
    local s, e = client:request("TestQueryDbState", { account_id = account_id })
    harness.assert(e == nil, "TestQueryDbState failed")
    return s
end

local function crud(client, account_id, params)
    params.account_id = account_id
    local ack, e = client:request("TestContainerCrud", params)
    if e ~= nil then
        harness.assert(false, "TestContainerCrud " .. params.op .. " failed: " .. tostring(e.detail))
    end
    harness.assert(ack.ok, params.op .. " outcome not Success: " .. tostring(ack.outcome))
    return ack
end

local dir = harness.data_dir("sync_jmap_container_crud")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-crud@example.test",
    display_name = "Sync JMAP CRUD",
    account_name = "Sync JMAP CRUD",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- Create a parent and a child folder.
local parent = crud(client, account.account_id, { op = "folder_create", name = "HarnessParent" })
local child = crud(client, account.account_id, { op = "folder_create", name = "HarnessChild" })

resync(client, account.account_id, "post-create")
local state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessParent") ~= nil, "parent folder missing after create")
harness.assert(folder_by_name(state.folders, "HarnessChild") ~= nil, "child folder missing after create")

-- Move the child under the parent.
crud(client, account.account_id, { op = "folder_move", id = child.new_id, parent = parent.new_id })
resync(client, account.account_id, "post-move")
state = db_state(client, account.account_id)
local moved = folder_by_name(state.folders, "HarnessChild")
harness.assert(moved ~= nil, "child folder missing after move")
harness.assert_eq(moved.parent_id, parent.new_id, "child folder did not reparent on the server")

-- Rename the child.
crud(client, account.account_id, { op = "folder_rename", id = child.new_id, name = "HarnessRenamed" })
resync(client, account.account_id, "post-rename")
state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessRenamed") ~= nil, "renamed folder missing on the server")
harness.assert(folder_by_name(state.folders, "HarnessChild") == nil, "old folder name still present after rename")

-- Delete both folders.
crud(client, account.account_id, { op = "folder_delete", id = child.new_id })
crud(client, account.account_id, { op = "folder_delete", id = parent.new_id })
resync(client, account.account_id, "post-delete")
state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessRenamed") == nil, "child folder still present after delete")
harness.assert(folder_by_name(state.folders, "HarnessParent") == nil, "parent folder still present after delete")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
