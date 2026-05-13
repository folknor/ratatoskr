-- description: JMAP secondary Mailbox/set create imports as a local folder
-- @covers: architecture.folder_vs_label_semantics_are_explicit
-- @covers: glossary.folders_labels.folder_rows_are_containers
-- @covers: glossary.folders_labels.labels_table_discriminates_folders_and_labels
-- @covers: glossary.folders_labels.non_system_ids_keep_provider_prefixes
-- @covers: glossary.folders_labels.provider_terms_translate_to_folder_label_semantics
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

local dir = harness.data_dir("sync_jmap_mailbox_secondary_create_import")
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
            isSubscribed = true,
        },
    },
}, "c1")
harness.assert(created.created ~= nil, "mailbox create missing created map")
harness.assert(created.created.scratch ~= nil, "mailbox create missing scratch result")
local remote_id = created.created.scratch.id
harness.assert(remote_id ~= nil, "created mailbox missing server id")

harness.clear_mock_requests(jmap_endpoint)
harness.marker("SYNC_START")
run_sync(client, account.account_id, "delta secondary")
harness.marker("SYNC_END")

local after = query_state(client, account.account_id, "after mailbox create")
local label_id = "jmap-" .. remote_id
local label = label_by_id(after, label_id)
harness.assert(label ~= nil, "created mailbox label missing")
harness.assert_eq(label.account_id, account.account_id, "created mailbox label account")
harness.assert_eq(label.name, "Secondary Scratch", "created mailbox label name")
harness.assert_eq(label.label_kind, "container", "created mailbox label kind")
harness.assert_eq(label.parent_label_id, nil, "created mailbox parent")
harness.assert_eq(label.is_subscribed, true, "created mailbox subscription flag")
harness.assert_eq(after.label_count, before.label_count + 1, "label count after mailbox create")

local requests = harness.mock_requests(jmap_endpoint, { stable = true })
harness.assert(
    count_account_requests(requests, "Mailbox/changes", "account-secondary") >= 1,
    "secondary delta did not check secondary Mailbox/changes"
)
harness.assert(
    count_account_requests(requests, "Mailbox/get", "account-secondary") >= 1,
    "secondary delta did not fetch secondary mailboxes"
)

harness.write_summary({
    correct = 1,
    target_account = "account-secondary",
    remote_mailbox_id = remote_id,
    local_label_id = label_id,
    label_count = after.label_count,
    provider_requests = #requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
