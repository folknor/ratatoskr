-- description: Gmail initial sync imports fixture mail, labels, folders, and attachments
-- @covers: glossary.folders_labels.provider_terms_translate_to_folder_label_semantics
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local function folder_by_id(folders, id)
    for _, folder in ipairs(folders) do
        if folder.id == id then
            return folder
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

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-initial-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local gmail_endpoint = harness.env("RATATOSKR_TEST_GMAIL_ENDPOINT")
harness.assert(gmail_endpoint ~= nil, "RATATOSKR_TEST_GMAIL_ENDPOINT missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-initial@example.test",
    display_name = "Sync Gmail",
    account_name = "Sync Gmail",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-initial-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
    attachment_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "message count")
harness.assert(state.thread_count >= 1, "thread count")

local synced_account = account_by_id(state, account.account_id)
harness.assert(synced_account ~= nil, "account missing after sync")
harness.assert(synced_account.initial_sync_completed, "initial sync did not mark account completed")

local inbox = folder_by_id(state.folders, "INBOX")
harness.assert(inbox ~= nil, "missing INBOX folder")
harness.assert_eq(inbox.name, "INBOX", "INBOX folder name")
harness.assert(folder_by_id(state.folders, "IMPORTANT") ~= nil, "missing IMPORTANT folder")

local harness_label = label_by_id(state.labels, "harness-label")
harness.assert(harness_label ~= nil, "missing harness-label")
harness.assert_eq(harness_label.name, "Harness", "harness-label display name")

harness.assert(message_by_subject(state.messages, "Hello") ~= nil, "missing Hello")
harness.assert(message_by_subject(state.messages, "Re: Hello") ~= nil, "missing Re: Hello")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local profile_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/profile")
local label_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/labels")
local message_list_requests =
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/messages")
local message_get_requests =
    harness.request_count_prefix(requests, "gmail", "GET /gmail/v1/users/me/messages/")
harness.assert(profile_requests >= 1, "Gmail sync did not fetch profile")
harness.assert(label_requests >= 1, "Gmail sync did not fetch labels")
harness.assert(message_list_requests >= 1, "Gmail sync did not list messages")
harness.assert(message_get_requests >= 1, "Gmail sync did not hydrate messages")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    thread_count = state.thread_count,
    folder_count = state.folder_count,
    label_count = state.label_count,
    provider_requests = #requests,
    gmail_profile_requests = profile_requests,
    gmail_label_requests = label_requests,
    gmail_message_list_requests = message_list_requests,
    gmail_message_get_requests = message_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
