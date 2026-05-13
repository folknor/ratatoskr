-- description: Gmail SendAs signature sync writes each account's aliases under its own bearer token without leakage
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
            code = "harness-gmail-" .. account_id,
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(
        response.access_token ~= nil,
        label .. " /oauth/token did not return access_token"
    )
    return response.access_token
end

local function signature_by_server_id(signatures, server_id)
    for _, sig in ipairs(signatures) do
        if sig.server_id == server_id then
            return sig
        end
    end
    return nil
end

local function imported_signatures(signatures)
    local out = {}
    for _, sig in ipairs(signatures) do
        if sig.server_id ~= nil and sig.server_id ~= "" then
            out[#out + 1] = sig
        end
    end
    return out
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)

-- Mint one bearer per fixture account so saehrimnir's Gmail layer can
-- attribute /gmail/v1/users/me/settings/sendAs to the right account.
local primary_token = mint_token(token_url, "account-primary", "primary")
local secondary_token = mint_token(token_url, "account-secondary", "secondary")
harness.assert(primary_token ~= secondary_token, "token store returned duplicate strings")

local dir = harness.data_dir("sync_gmail_send_as_multi_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local future_expiry = 2000000000

local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "Gmail Primary",
    account_name = "Gmail Primary",
    provider = "gmail_api",
    access_token = primary_token,
    refresh_token = "primary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "Gmail Secondary",
    account_name = "Gmail Secondary",
    provider = "gmail_api",
    access_token = secondary_token,
    refresh_token = "secondary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
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

local primary_state, primary_state_err = client:request("TestQueryDbState", {
    account_id = primary.account_id,
})
harness.assert(primary_state_err == nil, "primary TestQueryDbState failed")
-- TestSeedAccount also inserts a local-only "Harness" signature with
-- server_id = NULL; only Gmail-imported rows carry server_id, so the
-- imported set is what we assert on.
local primary_imported = imported_signatures(primary_state.signatures)
harness.assert_eq(#primary_imported, 2, "primary imported signature count")

local primary_main = signature_by_server_id(primary_state.signatures, "primary@example.com")
harness.assert(primary_main ~= nil, "primary missing main SendAs signature")
harness.assert_eq(
    primary_main.body_html,
    "<p>Primary signature -- send from <b>primary@example.com</b>.</p>",
    "primary main signature HTML"
)
harness.assert(
    string.find(primary_main.name, "Primary Owner", 1, true) ~= nil,
    "primary main signature display name in `name`"
)
harness.assert_eq(
    primary_main.is_default,
    true,
    "primary main signature is_default mirrors Gmail isDefault"
)
harness.assert_eq(
    primary_main.source,
    "gmail_sync",
    "primary main signature source"
)
harness.assert(
    primary_main.server_html_hash ~= nil and primary_main.server_html_hash ~= "",
    "primary main signature server_html_hash populated"
)

local primary_alias = signature_by_server_id(
    primary_state.signatures,
    "primary-alias@example.com"
)
harness.assert(primary_alias ~= nil, "primary missing alias SendAs signature")
harness.assert_eq(
    primary_alias.body_html,
    "<p>Primary alias signature.</p>",
    "primary alias signature HTML"
)
harness.assert_eq(
    primary_alias.is_default,
    false,
    "primary alias signature is_default false"
)

-- Cross-account leakage check: primary must NOT have secondary's identity.
harness.assert(
    signature_by_server_id(primary_state.signatures, "secondary@example.com") == nil,
    "primary leaked secondary's SendAs signature"
)

local secondary_state, secondary_state_err = client:request("TestQueryDbState", {
    account_id = secondary.account_id,
})
harness.assert(secondary_state_err == nil, "secondary TestQueryDbState failed")
local secondary_imported = imported_signatures(secondary_state.signatures)
harness.assert_eq(#secondary_imported, 1, "secondary imported signature count")

local secondary_main = signature_by_server_id(
    secondary_state.signatures,
    "secondary@example.com"
)
harness.assert(secondary_main ~= nil, "secondary missing main SendAs signature")
harness.assert_eq(
    secondary_main.body_html,
    "<p>Secondary signature -- different account.</p>",
    "secondary signature HTML"
)
harness.assert_eq(
    secondary_main.is_default,
    true,
    "secondary signature is_default"
)
harness.assert(
    signature_by_server_id(secondary_state.signatures, "primary@example.com") == nil,
    "secondary leaked primary's SendAs signature"
)
harness.assert(
    signature_by_server_id(secondary_state.signatures, "primary-alias@example.com") == nil,
    "secondary leaked primary's alias SendAs signature"
)

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local send_as_list_requests = harness.request_count(
    requests,
    "gmail",
    "GET /gmail/v1/users/me/settings/sendAs"
)
harness.assert(
    send_as_list_requests >= 2,
    "expected one sendAs list call per account, got " .. tostring(send_as_list_requests)
)

harness.write_summary({
    correct = 1,
    primary_imported_count = #primary_imported,
    secondary_imported_count = #secondary_imported,
    gmail_send_as_list_requests = send_as_list_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
