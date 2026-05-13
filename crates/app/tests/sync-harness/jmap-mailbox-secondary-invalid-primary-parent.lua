-- description: JMAP secondary Mailbox/set rejects primary-account parent ids
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

local function label_by_name(state, name)
    for _, label in ipairs(state.labels) do
        if label.name == name then
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

local dir = harness.data_dir("sync_jmap_mailbox_secondary_invalid_primary_parent")
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
local before = query_state(client, account.account_id, "before invalid mailbox create")

local rejected = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    create = {
        bad = {
            name = "Bad Cross Account Child",
            parentId = "mbx-primary-inbox",
        },
    },
}, "c1")
harness.assert(rejected.notCreated ~= nil, "invalid create missing notCreated map")
harness.assert(rejected.notCreated.bad ~= nil, "invalid create missing bad result")
harness.assert_eq(rejected.notCreated.bad.type, "invalidProperties", "invalid create error type")
harness.assert(rejected.created == nil or rejected.created.bad == nil, "invalid create unexpectedly succeeded")

run_sync(client, account.account_id, "delta secondary after rejected create")
local after = query_state(client, account.account_id, "after invalid mailbox create")
harness.assert_eq(after.label_count, before.label_count, "label count changed after rejected create")
harness.assert(
    label_by_name(after, "Bad Cross Account Child") == nil,
    "rejected cross-account child appeared in labels"
)

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    label_count = after.label_count,
    rejection_type = rejected.notCreated.bad.type,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
