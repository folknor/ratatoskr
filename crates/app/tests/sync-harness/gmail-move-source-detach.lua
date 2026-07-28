-- description: Gmail source-carrying moves detach the source in one batchModify
-- expected: pass
-- fixture: gmail-move-sources.toml
-- protocol: gmail
-- ceiling: 120s
--
-- Gmail's move patch is destination-only and a message id carries no source
-- label, so the source must ride `bulk_move_from` into the SAME `batchModify`
-- as the destination. This gate pins both halves of that contract:
--
--  1. BEHAVIOUR: after a restore-from-trash (MoveToFolder INBOX <- TRASH) and
--     an un-spam (SetSpam false: INBOX destination, whose patch removes
--     nothing by itself), a resync from the mock shows the source label GONE.
--     This is the assertion bifrost's move read-back guard deliberately does
--     not make - it reconciles destination membership only.
--  2. SHAPE: each move is exactly ONE bulk request. No per-id
--     `messages/{id}/modify` detach follows the batch - the 1+N shape this
--     gate exists to keep dead.

local function message_by_subject(messages, subject)
    for _, message in ipairs(messages) do
        if message.subject == subject then
            return message
        end
    end
    return nil
end

local function has_label(labels, expected)
    for _, label in ipairs(labels) do
        if label == expected then
            return true
        end
    end
    return false
end

local function read_thread(client, account_id, thread_id, label)
    local state, err = client:request("TestThreadRead", {
        account_id = account_id,
        thread_id = thread_id,
    })
    harness.assert(err == nil, label .. " TestThreadRead failed")
    return state
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-move-sources",
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(response.access_token ~= nil, "/oauth/token did not return access_token")
    return response.access_token
end

local function wait_for_action_completed(queue, plan_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local event = queue:recv(1)
        if event ~= nil and event.type == "ActionCompleted" then
            if event.plan_id == plan_id then
                return event
            end
        end
    end
    return nil
end

local function execute_action(client, queue, account_id, thread_id, operation, fields)
    local op = {
        account_id = account_id,
        thread_id = thread_id,
        operation = operation,
    }
    for key, value in pairs(fields or {}) do
        op[key] = value
    end
    local ack, ack_err = client:request("ActionExecutePlan", {
        operations = { [1] = op },
    })
    harness.assert(ack_err == nil, operation .. " action.execute_plan failed")
    harness.assert(ack.journaled, operation .. " plan was not journaled")

    local completed = wait_for_action_completed(queue, ack.plan_id, 15)
    harness.assert(completed ~= nil, operation .. " missing action.completed")
    harness.assert_eq(completed.summary_total, 1, operation .. " summary total")
    harness.assert_eq(completed.summary_remote_failed, 0, operation .. " remote failures")
    harness.assert_eq(completed.summary_local_only, 0, operation .. " degraded to local-only")
    harness.assert(
        completed.summary_remote_succeeded >= 1,
        operation .. " did not report remote success"
    )
    return completed
end

-- Every source-carrying move must be ONE bulk request: `batchModify` present,
-- and no other POST on the messages surface (a per-id `{id}/modify` detach
-- would land in the prefix count without landing in the batch count).
local function assert_single_batch_shape(admin_endpoint, operation)
    local requests = harness.mock_requests(admin_endpoint, { stable = true })
    local batch_modify = harness.request_count(
        requests,
        "gmail",
        "POST /gmail/v1/users/me/messages/batchModify"
    )
    local message_posts = harness.request_count_prefix(
        requests,
        "gmail",
        "POST /gmail/v1/users/me/messages"
    )
    harness.assert(batch_modify >= 1, operation .. " did not ride batchModify")
    harness.assert_eq(
        message_posts,
        batch_modify,
        operation .. " issued per-id message mutations besides the batch"
    )
end

local function resync(client, account_id, label)
    local result, err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(err == nil, label .. " resync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " resync"))
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_move_source_detach")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-move-sources@example.test",
    display_name = "Sync Gmail Move Sources",
    account_name = "Sync Gmail Move Sources",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-move-sources-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial_sync, initial_sync_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_sync_err == nil, "initial start_sync failed")
harness.assert_eq(
    initial_sync.result,
    "completed",
    initial_sync.error or "initial sync result"
)

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "initial message count")
local trashed = message_by_subject(state.messages, "Trashed")
harness.assert(trashed ~= nil, "missing Trashed")
local spammed = message_by_subject(state.messages, "Spammed")
harness.assert(spammed ~= nil, "missing Spammed")

-- Restore from trash: MoveToFolder with an explicit TRASH source. The INBOX
-- destination patch removes nothing on its own, so without the folded source
-- the thread comes back from the next sync still labelled TRASH.
harness.clear_mock_requests(admin_endpoint)
execute_action(client, queue, account.account_id, trashed.thread_id, "MoveToFolder", {
    dest = "INBOX",
    source = "TRASH",
})
assert_single_batch_shape(admin_endpoint, "MoveToFolder")
resync(client, account.account_id, "MoveToFolder")
local restored =
    read_thread(client, account.account_id, trashed.thread_id, "after MoveToFolder resync")
harness.assert(restored.exists, "thread missing after MoveToFolder resync")
harness.assert(
    has_label(restored.label_ids, "INBOX"),
    "restored thread not in inbox after resync"
)
harness.assert(
    not has_label(restored.label_ids, "TRASH"),
    "restored thread still labelled TRASH after resync - the source did not detach"
)

-- Un-spam: the role-shaped source (SPAM) with an INBOX destination. The
-- original regression shape: the destination-only patch removes nothing, so
-- SPAM sticks unless the source rides the same batchModify.
harness.clear_mock_requests(admin_endpoint)
execute_action(client, queue, account.account_id, spammed.thread_id, "SetSpam", { to = false })
assert_single_batch_shape(admin_endpoint, "SetSpam(false)")
resync(client, account.account_id, "SetSpam(false)")
local unspammed =
    read_thread(client, account.account_id, spammed.thread_id, "after un-spam resync")
harness.assert(unspammed.exists, "thread missing after un-spam resync")
harness.assert(
    has_label(unspammed.label_ids, "INBOX"),
    "un-spammed thread not in inbox after resync"
)
harness.assert(
    not has_label(unspammed.label_ids, "SPAM"),
    "un-spammed thread still labelled SPAM after resync - the source did not detach"
)

harness.write_summary({
    correct = 1,
    restored_labels = #restored.label_ids,
    unspammed_labels = #unspammed.label_ids,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
