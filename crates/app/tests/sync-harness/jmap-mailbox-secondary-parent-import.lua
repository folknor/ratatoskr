-- description: JMAP secondary Mailbox/set create preserves same-account parent binding
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: multi-account-secondary-primary.toml
-- protocol: jmap
-- ceiling: 120s

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

local function label_by_id(state, id)
    for _, label in ipairs(state.labels) do
        if label.id == id then
            return label
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

local dir = harness.data_dir("sync_jmap_mailbox_secondary_parent_import")
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

local created = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    create = {
        child = {
            name = "Secondary Child",
            parentId = "mbx-secondary-inbox",
            isSubscribed = true,
        },
    },
}, "c1")
harness.assert(created.created ~= nil, "mailbox create missing created map")
harness.assert(created.created.child ~= nil, "mailbox create missing child result")
local remote_id = created.created.child.id
harness.assert(remote_id ~= nil, "created mailbox missing server id")

run_sync(client, account.account_id, "delta secondary")

local after = query_state(client, account.account_id, "after mailbox create")
local label = label_by_id(after, "jmap-" .. remote_id)
harness.assert(label ~= nil, "created child mailbox label missing")
harness.assert_eq(label.name, "Secondary Child", "created child label name")
harness.assert_eq(label.account_id, account.account_id, "created child label account")
harness.assert_eq(label.parent_label_id, "INBOX", "created child parent label")
harness.assert_eq(label.label_kind, "container", "created child label kind")

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    remote_mailbox_id = remote_id,
    parent_label_id = label.parent_label_id,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
