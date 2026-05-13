-- description: Gmail sync scopes labels, threads, and messages by the bearer token's account_id
-- @covers: glossary.folders_labels.label_identity_is_account_scoped
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: gmail
-- ceiling: 120s

local function mint_token(token_url, account_id, label)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = account_id,
            code = "harness-gmail-oauth-" .. account_id,
            client_id = "ratatoskr-gmail-oauth-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(
        response.access_token ~= nil,
        label .. " /oauth/token did not return access_token"
    )
    return response.access_token
end

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function label_by_id(labels, id)
    for _, label in ipairs(labels) do
        if label.id == id then
            return label
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)

local primary_token = mint_token(token_url, "account-primary", "primary")
local secondary_token = mint_token(token_url, "account-secondary", "secondary")
harness.assert(primary_token ~= secondary_token, "token store returned duplicate strings")

local dir = harness.data_dir("sync_gmail_oauth_multi_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local future_expiry = 2000000000

local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "Gmail OAuth Primary",
    account_name = "Gmail OAuth Primary",
    provider = "gmail_api",
    access_token = primary_token,
    refresh_token = "primary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-oauth-harness",
    oauth_token_url = token_url,
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "Gmail OAuth Secondary",
    account_name = "Gmail OAuth Secondary",
    provider = "gmail_api",
    access_token = secondary_token,
    refresh_token = "secondary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-oauth-harness",
    oauth_token_url = token_url,
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

local function run_sync(account_id, label)
    local result, sync_err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

run_sync(primary.account_id, "primary")
run_sync(secondary.account_id, "secondary")

local function query(account_id, label)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 10,
    })
    harness.assert(state_err == nil, label .. " TestQueryDbState failed")
    return state
end

-- Primary: only sees its own thread/message; secondary's must not
-- have leaked even though both syncs run against the same /me/ paths.
local primary_state = query(primary.account_id, "primary")
harness.assert_eq(primary_state.message_count, 1, "primary message count")
harness.assert(
    message_by_subject(primary_state.messages, "Hello primary") ~= nil,
    "primary missing its own inbox email"
)
harness.assert(
    message_by_subject(primary_state.messages, "Hello secondary") == nil,
    "primary leaked secondary's inbox email"
)

local secondary_state = query(secondary.account_id, "secondary")
harness.assert_eq(secondary_state.message_count, 1, "secondary message count")
harness.assert(
    message_by_subject(secondary_state.messages, "Hello secondary") ~= nil,
    "secondary missing its own inbox email"
)
harness.assert(
    message_by_subject(secondary_state.messages, "Hello primary") == nil,
    "secondary leaked primary's inbox email"
)

-- Labels are keyed by (account_id, id) so well-known names like INBOX
-- legitimately repeat across accounts. The leakage check is therefore
-- on the account_id column itself: when we query for primary's
-- account_id, every returned label row must carry that account_id.
local function assert_labels_scoped(state, expected_account_id, label)
    harness.assert(#state.labels >= 1, label .. " missing imported labels")
    for _, lbl in ipairs(state.labels) do
        harness.assert_eq(
            lbl.account_id,
            expected_account_id,
            label .. " label " .. lbl.id .. " has wrong account_id"
        )
    end
    local inbox = label_by_id(state.labels, "INBOX")
    harness.assert(inbox ~= nil, label .. " missing canonical INBOX label")
    harness.assert_eq(
        inbox.account_id,
        expected_account_id,
        label .. " INBOX label has wrong account_id"
    )
    harness.assert_eq(inbox.label_kind, "container", label .. " INBOX label_kind")
end

assert_labels_scoped(primary_state, primary.account_id, "primary")
assert_labels_scoped(secondary_state, secondary.account_id, "secondary")

-- Mock-request log shows the same /gmail/v1/users/me/threads endpoint
-- was hit at least once per account - the bearer header is the only
-- thing distinguishing them.
local requests = harness.mock_requests(admin_endpoint, { stable = true })
local thread_list_requests = harness.request_count_prefix(
    requests,
    "gmail",
    "GET /gmail/v1/users/me/threads"
)
harness.assert(
    thread_list_requests >= 2,
    "expected at least one Gmail thread-list call per account"
)
local label_list_requests = harness.request_count(
    requests,
    "gmail",
    "GET /gmail/v1/users/me/labels"
)
harness.assert(
    label_list_requests >= 2,
    "expected at least one Gmail label-list call per account"
)

harness.write_summary({
    correct = 1,
    primary_message_count = primary_state.message_count,
    secondary_message_count = secondary_state.message_count,
    primary_label_count = #primary_state.labels,
    secondary_label_count = #secondary_state.labels,
    gmail_thread_list_requests = thread_list_requests,
    gmail_label_list_requests = label_list_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
