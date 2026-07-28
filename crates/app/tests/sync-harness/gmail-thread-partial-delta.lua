-- description: Gmail partial history delta on ONE message of a multi-message thread leaves its siblings intact
-- @covers: glossary.folders_labels.system_folder_ids_are_canonical
-- expected: pass
-- fixture: gmail-incremental.lua
-- protocol: gmail
-- ceiling: 120s
--
-- The gap this closes. Gmail's `history.list` reports a label change per
-- MESSAGE id, not per thread. A delta that stars email-003a therefore hands
-- the consumer a batch containing exactly ONE of thread-003's two messages,
-- while the thread-level projection (thread flags, thread_folders /
-- thread_labels membership) is a union over ALL of the thread's messages.
--
-- A recompute that derived the thread's state from the DELTA BATCH rather
-- than from every persisted thread-message row would silently drop
-- email-003b: its row, its thread membership, or the folder membership it
-- contributes. The per-message recompute makes that correct by construction
-- (it re-reads every persisted message of the touched thread), and the unit
-- side is covered by `gmail_consumer_membership_equals_legacy`. This is the
-- end-to-end proof that it stays that way through a real Gmail history walk.
--
-- What must hold after the partial delta:
--   * email-003b's row survives (message_count unchanged over the step).
--   * No thread splits or duplicates (thread_count unchanged over the step).
--   * email-003b still belongs to the SAME thread as email-003a.
--   * The star lands on email-003a ALONE - a whole-thread fan-out of the
--     delta's flags would star email-003b too, which is the mirror-image
--     bug of dropping it.
--   * The thread rolls the star up (union) and keeps its INBOX membership.

local function message_by_id(state, id)
    for _, message in ipairs(state.messages) do
        if message.id == id then
            return message
        end
    end
    return nil
end

local function assert_has_value(values, expected, message)
    for _, value in ipairs(values) do
        if value == expected then
            return
        end
    end
    harness.assert(false, message)
end

local function query_state(client, account_id)
    local state, err = client:request("TestQueryDbState", {
        account_id = account_id,
        message_limit = 20,
    })
    harness.assert(err == nil, "TestQueryDbState failed")
    return state
end

local function run_delta(client, account_id, label)
    local result, err = client:start_sync({
        account_id = account_id,
    }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

-- Steps are cursor-driven and strictly ordered, so the three earlier steps
-- are walked (and drained) to reach the one this script exists for.
local function apply_step(endpoint, step_id)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "test/fixture/step"),
        body = {
            expect = step_id,
        },
    })
    harness.assert(response.ok, "fixture step " .. step_id .. " failed")
    harness.assert_eq(response.step, step_id, "fixture step id")
    harness.assert_eq(response.applied, 1, "fixture step applied count")
    return response
end

local function mint_token(token_url)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = "account-1",
            code = "harness-gmail-thread-partial-delta-account-1",
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
local access_token = mint_token(token_url)

local dir = harness.data_dir("sync_gmail_thread_partial_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-thread-partial@example.test",
    display_name = "Sync Gmail Thread Partial",
    account_name = "Sync Gmail Thread Partial",
    provider = "gmail_api",
    access_token = access_token,
    refresh_token = "gmail-thread-partial-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- ── Baseline: the two-message thread imports as ONE thread ──────────
run_delta(client, account.account_id, "initial")
local initial = query_state(client, account.account_id)
harness.assert_eq(initial.message_count, 4, "initial message count")

local sibling_a = message_by_id(initial, "email-003a")
local sibling_b = message_by_id(initial, "email-003b")
harness.assert(sibling_a ~= nil, "email-003a missing after initial sync")
harness.assert(sibling_b ~= nil, "email-003b missing after initial sync")
harness.assert_eq(sibling_b.subject, "Re: Lunch?", "email-003b subject")
harness.assert_eq(
    sibling_b.thread_id,
    sibling_a.thread_id,
    "Gmail's two-message thread did not import as one thread"
)
harness.assert(not sibling_a.is_starred, "email-003a starred before the delta")
harness.assert(not sibling_b.is_starred, "email-003b starred before the delta")

local thread_id = sibling_a.thread_id
local thread_before, thread_before_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread_id,
})
harness.assert(thread_before_err == nil, "TestThreadRead before the delta failed")
harness.assert(thread_before.exists, "thread missing before the delta")
harness.assert(not thread_before.is_starred, "thread starred before the delta")
assert_has_value(thread_before.label_ids, "INBOX", "thread lacked INBOX before the delta")

-- ── Walk the earlier steps so the cursor reaches `thread-label` ─────
apply_step(admin_endpoint, "new")
run_delta(client, account.account_id, "new step")
apply_step(admin_endpoint, "delete")
run_delta(client, account.account_id, "delete step")
apply_step(admin_endpoint, "label")
run_delta(client, account.account_id, "label step")

local before = query_state(client, account.account_id)
harness.assert(
    message_by_id(before, "email-003b") ~= nil,
    "email-003b lost while walking the earlier steps"
)

-- ── The scenario: a history record naming email-003a ALONE ──────────
harness.clear_mock_requests(admin_endpoint)
local partial_step = apply_step(admin_endpoint, "thread-label")
harness.assert_eq(
    partial_step.changes.emails.updated[1],
    "email-003a",
    "partial-delta step must update exactly one message of the thread"
)
run_delta(client, account.account_id, "partial thread delta")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(
    harness.request_count(requests, "gmail", "GET /gmail/v1/users/me/history") >= 1,
    "partial thread delta did not walk Gmail history"
)

local after = query_state(client, account.account_id)
harness.assert_eq(
    after.message_count,
    before.message_count,
    "a partial thread delta dropped or duplicated a message row"
)
harness.assert_eq(
    after.thread_count,
    before.thread_count,
    "a partial thread delta split or duplicated a thread"
)

local acted = message_by_id(after, "email-003a")
local sibling = message_by_id(after, "email-003b")
harness.assert(acted ~= nil, "email-003a missing after the partial delta")
harness.assert(sibling ~= nil, "email-003b did not survive its sibling's delta")
harness.assert_eq(
    sibling.thread_id,
    acted.thread_id,
    "email-003b was re-threaded away from its sibling"
)
harness.assert_eq(sibling.subject, "Re: Lunch?", "email-003b row was rewritten")
harness.assert(acted.is_starred, "the delta's own message did not import the star")
harness.assert(
    not sibling.is_starred,
    "the star fanned out to the thread's other message; the delta named email-003a only"
)
harness.assert(sibling.is_read, "email-003b lost its read state")

local thread_after, thread_after_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = thread_id,
})
harness.assert(thread_after_err == nil, "TestThreadRead after the partial delta failed")
harness.assert(thread_after.exists, "thread vanished after the partial delta")
harness.assert(thread_after.is_starred, "thread did not roll up the one message's star")
assert_has_value(
    thread_after.label_ids,
    "INBOX",
    "thread lost the INBOX membership its messages still carry"
)

harness.write_summary({
    correct = 1,
    message_count = after.message_count,
    thread_count = after.thread_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
