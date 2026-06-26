-- description: Gmail sync triggers PrefetchRuntime; attachment bytes land in PackStore
-- expected: pass
-- fixture: jmap-attach.toml
-- protocol: gmail
-- ceiling: 120s

-- Phase 7 (attachments roadmap) provider parity: with the `provider =
-- 'jmap'` filter lifted from the post-sync sweep and boot recovery
-- kick, a Gmail account's attachments must also land in PackStore by
-- the time `prefetch.completed` fires.

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local function wait_for_prefetch_completed(queue, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "prefetch.completed"
        then
            return notification
        end
    end
    return nil
end

local function wait_for_content_hash(client, account_id, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
            attachment_limit = 10,
        })
        harness.assert(state_err == nil, "TestQueryDbState failed")
        local row = attachment_by_filename(state.attachments, "sample.txt")
        if row ~= nil and row.content_hash ~= nil then
            return row
        end
        harness.sleep(250)
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
            code = "harness-gmail-prefetch-account-1",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_attachment_prefetch")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-prefetch@example.test",
    display_name = "Sync Gmail Prefetch",
    account_name = "Sync Gmail Prefetch",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-prefetch-refresh-unused",
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

harness.marker("PREFETCH_WAIT_START")
local prefetch_done = wait_for_prefetch_completed(queue, 30)
harness.marker("PREFETCH_WAIT_END")
harness.assert(prefetch_done ~= nil, "prefetch.completed not observed")
harness.assert((prefetch_done.fetched or 0) >= 1, "prefetch fetched count")
harness.assert_eq(prefetch_done.failed, 0, "prefetch failed count")

local row = wait_for_content_hash(client, account.account_id, 5)
harness.assert(row ~= nil, "attachment row missing content_hash after prefetch")
harness.assert(#row.content_hash == 64, "content_hash should be 32-byte BLAKE3 hex (64 chars)")

local packs_dir = dir .. "/attachment_packs"
harness.assert(harness.path_exists(packs_dir), "attachment_packs dir missing")
harness.assert(
    harness.dir_has_prefix(packs_dir, "data-"),
    "no pack file under attachment_packs/"
)

harness.write_summary({
    correct = 1,
    prefetch_fetched = prefetch_done.fetched,
    prefetch_skipped = prefetch_done.skipped,
    prefetch_failed = prefetch_done.failed,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
