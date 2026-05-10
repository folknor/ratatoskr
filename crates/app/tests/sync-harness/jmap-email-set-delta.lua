-- description: JMAP Email/set mutation is imported by delta sync
-- expected: pass
-- fixture: jmap-small.toml
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

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local jmap_api_url = harness.join_url(jmap_endpoint, "jmap/api")

local dir = harness.data_dir("sync_jmap_email_set_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-email-set@example.test",
    display_name = "Sync JMAP Email Set",
    account_name = "Sync JMAP Email Set",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(before_err == nil, "TestQueryDbState before mutation failed")
harness.assert_eq(before.message_count, 2, "initial message count")
local before_hello = message_by_subject(before, "Hello")
harness.assert(before_hello ~= nil, "missing Hello before mutation")
harness.assert_eq(before_hello.id, "email-001", "fixture email id")
harness.assert(not before_hello.is_read, "Hello unexpectedly read before mutation")

-- `account-1` and `email-001` are fixture IDs from jmap-small.toml.
-- The raw JMAP mutation intentionally targets the same message that
-- ratatoskr imported during the initial sync.
local mutation = harness.http_json({
    method = "POST",
    url = jmap_api_url,
    body = {
        using = {
            "urn:ietf:params:jmap:core",
            "urn:ietf:params:jmap:mail",
        },
        methodCalls = {
            {
                "Email/set",
                {
                    accountId = "account-1",
                    update = {
                        ["email-001"] = {
                            ["keywords/$seen"] = true,
                        },
                    },
                },
                "c0",
            },
        },
    },
})
harness.assert_eq(mutation.methodResponses[1][1], "Email/set", "mutation response method")
local mutation_body = mutation.methodResponses[1][2]
harness.assert(mutation_body.updated ~= nil, "mutation did not report updated map")
harness.assert(
    mutation_body.newState ~= mutation_body.oldState,
    "Email/set did not advance fixture state"
)

harness.clear_mock_requests(jmap_endpoint)

local delta, delta_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(delta_err == nil, "delta start_sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "delta sync result")

local requests = harness.mock_requests(jmap_endpoint)
harness.assert(
    harness.request_count(requests, "jmap", "Email/changes") >= 1,
    "delta sync did not call Email/changes"
)
harness.assert(
    harness.request_count(requests, "jmap", "Email/get") >= 1,
    "delta sync did not fetch updated email"
)
harness.assert_eq(
    harness.request_count(requests, "jmap", "Email/query"),
    0,
    "delta sync unexpectedly ran Email/query"
)

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_err == nil, "TestQueryDbState after delta failed")
harness.assert_eq(after.message_count, 2, "post-delta message count")
local after_hello = message_by_subject(after, "Hello")
harness.assert(after_hello ~= nil, "missing Hello after delta")
harness.assert(after_hello.is_read, "delta sync did not import remote read state")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
