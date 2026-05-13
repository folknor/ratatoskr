-- description: JMAP multi-account secondary sync ignores primary account mutations
-- expected: pass
-- fixture: multi-account-secondary-primary.toml
-- protocol: jmap
-- ceiling: 120s

local function message_by_subject(state, subject)
    for _, message in ipairs(state.messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

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

local function mutate_other_account(endpoint)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "jmap/api"),
        body = {
            using = {
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
            },
            methodCalls = {
                {
                    "Email/set",
                    {
                        accountId = "account-primary",
                        update = {
                            ["email-primary-001"] = {
                                ["keywords/$seen"] = true,
                            },
                        },
                    },
                    "c0",
                },
            },
        },
    })
    harness.assert_eq(response.methodResponses[1][1], "Email/set", "mutation method")
    local body = response.methodResponses[1][2]
    harness.assert(body.updated ~= nil, "primary mutation did not update")
    harness.assert(body.newState ~= body.oldState, "primary mutation did not advance state")
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
harness.clear_mock_requests(jmap_endpoint)

local dir = harness.data_dir("sync_jmap_multi_account_secondary_isolation")
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

local after_initial = query_state(client, account.account_id, "after initial")
harness.assert_eq(after_initial.message_count, 1, "initial message count")
harness.assert(message_by_subject(after_initial, "Hello secondary") ~= nil, "missing secondary email")
harness.assert(message_by_subject(after_initial, "Hello primary") == nil, "primary email leaked into secondary sync")

local initial_requests = harness.mock_requests(jmap_endpoint, { stable = true })
harness.assert(
    count_account_requests(initial_requests, "Mailbox/get", "account-secondary") >= 1,
    "initial sync did not request secondary mailboxes"
)
harness.assert(
    count_account_requests(initial_requests, "Email/query", "account-secondary") >= 1,
    "initial sync did not query secondary email"
)
harness.assert(
    count_account_requests(initial_requests, "Email/get", "account-secondary") >= 1,
    "initial sync did not fetch secondary email"
)

mutate_other_account(jmap_endpoint)
harness.clear_mock_requests(jmap_endpoint)

harness.marker("SYNC_START")
run_sync(client, account.account_id, "delta secondary")
harness.marker("SYNC_END")

local delta_requests = harness.mock_requests(jmap_endpoint, { stable = true })
harness.assert(
    count_account_requests(delta_requests, "Email/changes", "account-secondary") >= 1,
    "delta sync did not check secondary email changes"
)
harness.assert_eq(
    count_account_requests(delta_requests, "Email/changes", "account-primary"),
    0,
    "delta sync unexpectedly checked primary email changes"
)
harness.assert_eq(
    count_account_requests(delta_requests, "Email/get", "account-secondary"),
    0,
    "secondary delta fetched email after primary-only mutation"
)

local after_delta = query_state(client, account.account_id, "after delta")
harness.assert_eq(after_delta.message_count, 1, "delta message count")
harness.assert(message_by_subject(after_delta, "Hello secondary") ~= nil, "secondary email disappeared")
harness.assert(message_by_subject(after_delta, "Hello primary") == nil, "primary email leaked after delta")

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    message_count = after_delta.message_count,
    provider_requests = #delta_requests,
    email_changes_secondary = count_account_requests(delta_requests, "Email/changes", "account-secondary"),
    email_get_secondary = count_account_requests(delta_requests, "Email/get", "account-secondary"),
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
