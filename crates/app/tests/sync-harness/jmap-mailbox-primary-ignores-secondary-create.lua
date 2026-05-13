-- description: JMAP primary delta ignores secondary Mailbox/set creates
-- expected: pass
-- fixture: multi-account-small.toml
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

local function count_account_requests(requests, command, account_id)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == "jmap"
            and request.command == command
            and request.detail ~= nil
            and request.detail.account_id == account_id then
            count = count + 1
        end
    end
    return count
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
harness.clear_mock_requests(jmap_endpoint)

local dir = harness.data_dir("sync_jmap_mailbox_primary_ignores_secondary_create")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "JMAP Primary",
    account_name = "JMAP Primary",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

run_sync(client, account.account_id, "initial primary")
local before = query_state(client, account.account_id, "before secondary mailbox create")

local created = jmap_call(jmap_endpoint, "Mailbox/set", {
    accountId = "account-secondary",
    create = {
        scratch = {
            name = "Secondary Scratch",
            isSubscribed = true,
        },
    },
}, "c1")
harness.assert(created.created ~= nil, "secondary mailbox create missing created map")
harness.assert(created.created.scratch ~= nil, "secondary mailbox create missing scratch result")

harness.clear_mock_requests(jmap_endpoint)
harness.marker("SYNC_START")
run_sync(client, account.account_id, "delta primary")
harness.marker("SYNC_END")

local after = query_state(client, account.account_id, "after primary delta")
harness.assert_eq(after.label_count, before.label_count, "primary label count changed")
harness.assert(
    label_by_name(after, "Secondary Scratch") == nil,
    "secondary mailbox leaked into primary account labels"
)

local requests = harness.mock_requests(jmap_endpoint, { stable = true })
harness.assert(
    count_account_requests(requests, "Mailbox/changes", "account-primary") >= 1,
    "primary delta did not check primary Mailbox/changes"
)
harness.assert_eq(
    count_account_requests(requests, "Mailbox/changes", "account-secondary"),
    0,
    "primary delta unexpectedly checked secondary Mailbox/changes"
)

harness.write_summary({
    correct = 1,
    target_account = "account-primary",
    label_count = after.label_count,
    provider_requests = #requests,
    mailbox_changes_primary = count_account_requests(requests, "Mailbox/changes", "account-primary"),
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
