-- description: IMAP mailbox create/rename/delete round-trips through engine.container_*
-- expected: pass
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s
--
-- B6b container-CRUD gate (IMAP). A capability GAIN: the legacy ProviderOps
-- IMAP impl returned Failed for folder create/rename/delete, but bifrost-imap
-- implements mailbox CREATE/RENAME/DELETE, so the folder CRUD action handlers
-- now succeed. A follow-up resync's containers_list (mailbox LIST) must
-- reflect each step on the server.

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

local dir = harness.data_dir("sync_imap_container_crud")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-crud@example.test",
    display_name = "Sync IMAP CRUD",
    account_name = "Sync IMAP CRUD",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local mailbox = crud(client, account.account_id, { op = "folder_create", name = "HarnessBox" })
resync(client, account.account_id, "post-create")
local state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessBox") ~= nil, "mailbox missing after create")

crud(client, account.account_id, { op = "folder_rename", id = mailbox.new_id, name = "HarnessBoxRenamed" })
resync(client, account.account_id, "post-rename")
state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessBoxRenamed") ~= nil, "renamed mailbox missing on the server")
harness.assert(folder_by_name(state.folders, "HarnessBox") == nil, "old mailbox name still present after rename")

-- Rename reassigns the IMAP path (folder-{path}); delete by the renamed id.
local renamed = folder_by_name(state.folders, "HarnessBoxRenamed")
crud(client, account.account_id, { op = "folder_delete", id = renamed.id })
resync(client, account.account_id, "post-delete")
state = db_state(client, account.account_id)
harness.assert(folder_by_name(state.folders, "HarnessBoxRenamed") == nil, "mailbox still present after delete")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
