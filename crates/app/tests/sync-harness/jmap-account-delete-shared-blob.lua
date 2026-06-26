-- description: account.delete preserves shared blobs and tombstones unshared blobs (Phase 4 cross-account coverage)
-- expected: pass
-- fixture: multi-account-attach.toml
-- protocol: jmap
-- ceiling: 90s

-- AccountDeletionStep::AttachmentCache (Phase 4) tombstones a blob
-- only when the deleted account is its sole referencer; blobs still
-- referenced by surviving accounts must stay live.
-- multi-account-attach.toml seeds two accounts (alice, bob) sharing
-- one attachment via byte-identical data_path; alice also has an
-- unshared attachment. After deleting alice we assert:
--
--   shared content_hash:    present, tombstoned_at IS NULL    (bob still references)
--   unshared content_hash:  present, tombstoned_at NOT NULL   (no remaining referencer)

local function attachment_by_key(attachments, key)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == key
            or attachment.remote_attachment_id == key
            or attachment.id == key then
            return attachment
        end
    end
    return nil
end

local function wait_for_prefetch_completed(queue, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil and notification.method == "prefetch.completed" then
            return notification
        end
    end
    return nil
end

-- Poll TestQueryDbState until every named filename has its
-- content_hash populated. Mirrors wait_for_content_hash in
-- jmap-attachment-prefetch.lua: the prefetch.completed notification
-- is the primary signal but the DB row is the correctness contract.
local function wait_for_content_hashes(client, account_id, filenames, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
            attachment_limit = 10,
        })
        harness.assert(state_err == nil, "TestQueryDbState failed")
        local all_populated = true
        local rows = {}
        for _, key in ipairs(filenames) do
            local row = attachment_by_key(state.attachments, key)
            if row == nil or row.content_hash == nil then
                all_populated = false
                break
            end
            rows[key] = row
        end
        if all_populated then
            return rows
        end
        harness.sleep(250)
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)

-- Two ratatoskr-side JMAP accounts both syncing against saehrimnir
-- requires per-bearer routing through `oauth::account_from_bearer` -
-- basic-auth pins every connection to the fixture's `primary = true`
-- account regardless of credentials.
local function mint_token(account_id, label)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = account_id,
            code = "harness-jmap-shared-blob-" .. account_id,
            client_id = "ratatoskr-jmap-shared-blob-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil,
        label .. " /oauth/token did not return access_token")
    return response.access_token
end

local alice_token = mint_token("account-alice", "alice")
local bob_token = mint_token("account-bob", "bob")
harness.assert(alice_token ~= bob_token, "token store returned duplicate strings")

local dir = harness.data_dir("sync_jmap_account_delete_shared_blob")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local future_expiry = 2000000000

local alice, alice_err = client:request("TestSeedAccount", {
    email = "alice@example.com",
    display_name = "Alice",
    account_name = "Alice",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = alice_token,
    refresh_token = "alice-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-jmap-shared-blob-harness",
    oauth_token_url = token_url,
})
harness.assert(alice_err == nil, "alice TestSeedAccount failed")

local bob, bob_err = client:request("TestSeedAccount", {
    email = "bob@example.com",
    display_name = "Bob",
    account_name = "Bob",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = bob_token,
    refresh_token = "bob-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-jmap-shared-blob-harness",
    oauth_token_url = token_url,
})
harness.assert(bob_err == nil, "bob TestSeedAccount failed")

local function run_sync(account_id, label)
    local result, sync_err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(sync_err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

harness.marker("SYNC_ALICE_START")
run_sync(alice.account_id, "alice")
harness.marker("SYNC_ALICE_END")

harness.marker("SYNC_BOB_START")
run_sync(bob.account_id, "bob")
harness.marker("SYNC_BOB_END")

local bob_thread, bob_thread_err = client:request("TestSeedThread", {
    account_id = bob.account_id,
    thread_id = "bob-shared-thread",
    message_id = "bob-shared-message",
    subject = "Shared attachment - Bob local reference",
    body_text = "Bob keeps the shared blob alive.",
})
harness.assert(bob_thread_err == nil, "bob TestSeedThread failed")

local _, bob_attachment_err = client:request("TestSeedCachedAttachment", {
    account_id = bob.account_id,
    message_id = bob_thread.message_id,
    attachment_id = "blob-bob-shared",
    filename = "shared.txt",
    mime_type = "text/plain",
    content = "attachment payload for saehrimnir tests\n",
})
harness.assert(bob_attachment_err == nil, "bob TestSeedCachedAttachment failed")

-- Drain at least one prefetch.completed; the helper below polls for
-- the actual content_hash population which is the correctness signal.
wait_for_prefetch_completed(queue, 30)

local alice_rows = wait_for_content_hashes(
    client, alice.account_id, { "blob-alice-shared", "blob-alice-unshared" }, 10)
harness.assert(alice_rows ~= nil, "alice attachments never got content_hash populated")
local shared_row = alice_rows["blob-alice-shared"]
local unshared_row = alice_rows["blob-alice-unshared"]

local bob_rows = wait_for_content_hashes(
    client, bob.account_id, { "blob-bob-shared" }, 10)
harness.assert(bob_rows ~= nil, "bob's shared.txt never got content_hash populated")
local bob_shared = bob_rows["blob-bob-shared"]

local shared_hash = shared_row.content_hash
local unshared_hash = unshared_row.content_hash
harness.assert(shared_hash ~= unshared_hash, "shared and unshared hashes should differ")
harness.assert_eq(bob_shared.content_hash, shared_hash,
    "bob's shared blob should hash to the same content_hash as alice's")

local pre_shared, pre_shared_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = shared_hash,
})
harness.assert(pre_shared_err == nil, "pre-delete shared probe failed")
harness.assert(pre_shared.present, "shared blob should be present pre-delete")
harness.assert(pre_shared.tombstoned_at == nil, "shared blob should be live pre-delete")

local pre_unshared, pre_unshared_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = unshared_hash,
})
harness.assert(pre_unshared_err == nil, "pre-delete unshared probe failed")
harness.assert(pre_unshared.present, "unshared blob should be present pre-delete")
harness.assert(pre_unshared.tombstoned_at == nil, "unshared blob should be live pre-delete")

harness.marker("ACCOUNT_DELETE")
local _, delete_err = client:request("AccountDelete", {
    account_id = alice.account_id,
})
harness.assert(delete_err == nil, "alice account.delete failed: " .. tostring(delete_err))

local post_shared, post_shared_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = shared_hash,
})
harness.assert(post_shared_err == nil, "post-delete shared probe failed")
harness.assert(post_shared.present, "shared attachment_blobs row should survive alice delete")
harness.assert(post_shared.tombstoned_at == nil,
    "shared blob must NOT be tombstoned - bob still references (got tombstoned_at=" ..
    tostring(post_shared.tombstoned_at) .. ")")

local post_unshared, post_unshared_err = client:request("TestQueryBlobTombstoneState", {
    content_hash = unshared_hash,
})
harness.assert(post_unshared_err == nil, "post-delete unshared probe failed")
harness.assert(post_unshared.present, "unshared attachment_blobs row should survive alice delete")
harness.assert(post_unshared.tombstoned_at ~= nil,
    "unshared blob must be tombstoned after alice delete (got tombstoned_at=" ..
    tostring(post_unshared.tombstoned_at) .. ")")

harness.write_summary({
    correct = 1,
    shared_tombstoned_at = post_shared.tombstoned_at,
    unshared_tombstoned_at = post_unshared.tombstoned_at,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
