-- description: IMAP LOGIN routes each authenticated account to its own fixture mailboxes
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: imap
-- ceiling: 120s

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function run_sync(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local function query_messages(client, account_id, label)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(err == nil, label .. " TestQueryDbState failed")
    return state
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_imap_login_multi_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- TestSeedAccount wires imap_username = email, so the username the
-- ratatoskr client logs in with matches the fixture's account name.
-- saehrimnir's IMAP LOGIN handler looks up the fixture account by
-- username and binds the connection to that account for the rest of
-- the session.
local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "IMAP Primary",
    account_name = "IMAP Primary",
    provider = "imap",
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "IMAP Secondary",
    account_name = "IMAP Secondary",
    provider = "imap",
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

run_sync(client, primary.account_id, "primary")
run_sync(client, secondary.account_id, "secondary")

local primary_state = query_messages(client, primary.account_id, "primary")
harness.assert_eq(primary_state.message_count, 1, "primary message count")
harness.assert(
    message_by_subject(primary_state.messages, "Hello primary") ~= nil,
    "primary missing its own inbox email"
)
harness.assert(
    message_by_subject(primary_state.messages, "Hello secondary") == nil,
    "primary leaked secondary's inbox email"
)

local secondary_state = query_messages(client, secondary.account_id, "secondary")
harness.assert_eq(secondary_state.message_count, 1, "secondary message count")
harness.assert(
    message_by_subject(secondary_state.messages, "Hello secondary") ~= nil,
    "secondary missing its own inbox email"
)
harness.assert(
    message_by_subject(secondary_state.messages, "Hello primary") == nil,
    "secondary leaked primary's inbox email"
)

-- Saehrimnir scrubs LOGIN args from the request log (credentials would
-- otherwise leak verbatim), so we count by command rather than asserting
-- a per-account detail. One LOGIN per sync run is enough to prove the
-- LOGIN path was reached for each account.
local requests = harness.mock_requests(admin_endpoint, { stable = true })
local login_requests = harness.request_count(requests, "imap", "LOGIN")
local list_requests = harness.request_count(requests, "imap", "LIST")
local select_requests = harness.request_count(requests, "imap", "SELECT")
harness.assert(login_requests >= 2, "expected at least one LOGIN per account")
harness.assert(list_requests >= 2, "expected at least one LIST per account")
harness.assert(select_requests >= 2, "expected at least one SELECT per account")

harness.write_summary({
    correct = 1,
    primary_message_count = primary_state.message_count,
    secondary_message_count = secondary_state.message_count,
    imap_login_requests = login_requests,
    imap_list_requests = list_requests,
    imap_select_requests = select_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
