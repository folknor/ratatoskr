-- description: JMAP secondary Mailbox/set destroy reaps the local folder row
-- @covers: glossary.folders_labels.folder_rows_are_containers
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: multi-account-secondary-primary.toml
-- protocol: jmap
-- ceiling: 120s
--
-- The reconciliation half of the container sync: a mailbox created and then
-- DESTROYED on the server must not leave a stale `folders` row behind.
-- Exercises `reap_missing_personal_folders` end to end - the destroy arrives
-- as a `Type(Mailbox)` change batch, triggers the container re-snapshot, and
-- the reap deletes the row the snapshot no longer carries. System folder rows
-- (seeded outside the container sync) must survive the same reap.

local function query_state(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, label .. " TestQueryDbState failed")
    return state
end

local function run_sync(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local function folder_by_id(state, id)
    for _, folder in ipairs(state.folders) do
        if folder.id == id then
            return folder
        end
    end
    return nil
end

local function jmap_call(endpoint, method, args, call_id)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "jmap/api"),
        body = {
            using = {
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
            },
            methodCalls = {
                { method, args, call_id or "c0" },
            },
        },
    })
    harness.assert_eq(response.methodResponses[1][1], method, method .. " response method")
    return response.methodResponses[1][2]
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("sync_jmap_mailbox_secondary_destroy_reap")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "JMAP Secondary",
    account_name = "JMAP Secondary",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_sync(client, account.account_id, "initial secondary")
local before = query_state(client, account.account_id, "before mailbox create")

local created = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    create = {
        doomed = {
            name = "Secondary Doomed",
        },
    },
}, "c1")
harness.assert(created.created ~= nil, "mailbox create missing created map")
harness.assert(created.created.doomed ~= nil, "mailbox create missing doomed result")
local remote_id = created.created.doomed.id
harness.assert(remote_id ~= nil, "created mailbox missing server id")
local folder_id = "jmap-" .. remote_id

run_sync(client, account.account_id, "delta secondary create")
local after_create = query_state(client, account.account_id, "after mailbox create")
harness.assert(
    folder_by_id(after_create, folder_id) ~= nil,
    "created mailbox folder missing before destroy"
)
harness.assert_eq(
    after_create.folder_count,
    before.folder_count + 1,
    "folder count after mailbox create"
)

local destroyed = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    destroy = { remote_id },
}, "c2")
harness.assert(destroyed.destroyed ~= nil, "mailbox destroy missing destroyed list")
harness.assert_eq(destroyed.destroyed[1], remote_id, "mailbox destroy id")

run_sync(client, account.account_id, "delta secondary destroy")
local after_destroy = query_state(client, account.account_id, "after mailbox destroy")
harness.assert(
    folder_by_id(after_destroy, folder_id) == nil,
    "destroyed mailbox folder row was not reaped"
)
harness.assert_eq(
    after_destroy.folder_count,
    before.folder_count,
    "folder count back to baseline after destroy"
)
harness.assert(
    folder_by_id(after_destroy, "INBOX") ~= nil,
    "system INBOX row must survive the reap"
)
harness.assert(
    folder_by_id(after_destroy, "SENT") ~= nil,
    "seeded system SENT row must survive the reap"
)

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    remote_mailbox_id = remote_id,
    local_folder_id = folder_id,
    folder_count = after_destroy.folder_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
