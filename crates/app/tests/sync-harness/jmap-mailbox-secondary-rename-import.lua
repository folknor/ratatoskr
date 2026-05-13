-- description: JMAP secondary Mailbox/set rename updates the local folder row
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

local dir = harness.data_dir("sync_jmap_mailbox_secondary_rename_import")
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
        scratch = {
            name = "Secondary Scratch",
        },
    },
}, "c1")
harness.assert(created.created ~= nil, "mailbox create missing created map")
harness.assert(created.created.scratch ~= nil, "mailbox create missing scratch result")
local remote_id = created.created.scratch.id
harness.assert(remote_id ~= nil, "created mailbox missing server id")
local label_id = "jmap-" .. remote_id

run_sync(client, account.account_id, "delta secondary create")
local after_create = query_state(client, account.account_id, "after mailbox create")
local label = label_by_id(after_create, label_id)
harness.assert(label ~= nil, "created mailbox label missing")
harness.assert_eq(label.name, "Secondary Scratch", "created mailbox label name")

local renamed = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    update = {
        [remote_id] = {
            name = "Renamed Secondary Scratch",
        },
    },
}, "c2")
harness.assert(renamed.updated ~= nil, "mailbox rename missing updated map")

run_sync(client, account.account_id, "delta secondary rename")
local after_rename = query_state(client, account.account_id, "after mailbox rename")
local renamed_label = label_by_id(after_rename, label_id)
harness.assert(renamed_label ~= nil, "renamed mailbox label missing")
harness.assert_eq(renamed_label.name, "Renamed Secondary Scratch", "renamed mailbox label name")
harness.assert_eq(after_rename.label_count, before.label_count + 1, "label count after rename")

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    remote_mailbox_id = remote_id,
    local_label_id = label_id,
    label_count = after_rename.label_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
