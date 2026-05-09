-- description: IMAP steady-state sync avoids refetching unchanged messages
-- expected: pass
-- fixture: imap-small.toml
-- protocol: imap
-- ceiling: 120s

local function join_url(base, suffix)
    if string.sub(base, -1) == "/" then
        return base .. suffix
    end
    return base .. "/" .. suffix
end

local function account_by_id(state, account_id)
    for _, account in ipairs(state.accounts) do
        if account.id == account_id then
            return account
        end
    end
    return nil
end

local function request_count(requests, protocol, command)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == protocol and request.command == command then
            count = count + 1
        end
    end
    return count
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local requests_url = join_url(admin_endpoint, "test/requests")

local dir = harness.data_dir("sync_imap_steady_state_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-delta@example.test",
    display_name = "Sync IMAP Delta",
    account_name = "Sync IMAP Delta",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local first, first_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(first_err == nil, "initial start_sync failed")
harness.assert_eq(first.result, "completed", first.error or "initial sync result")

local after_initial, after_initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_initial_err == nil, "TestQueryDbState after initial sync failed")
harness.assert_eq(after_initial.message_count, 2, "initial message count")
harness.assert_eq(after_initial.unread_message_count, 1, "initial unread count")
local synced_account = account_by_id(after_initial, account.account_id)
harness.assert(synced_account ~= nil, "account missing after initial sync")
harness.assert(
    synced_account.initial_sync_completed,
    "initial sync did not mark account completed"
)

harness.http_delete(requests_url)

local second, second_err = client:start_sync({
    account_id = account.account_id,
}, 30)
harness.assert(second_err == nil, "delta start_sync failed")
harness.assert_eq(second.result, "completed", second.error or "delta sync result")

local requests = harness.http_get(requests_url)
harness.assert(
    request_count(requests, "imap", "LIST") >= 1,
    "delta sync did not list folders"
)
harness.assert(
    request_count(requests, "imap", "SELECT") >= 1,
    "delta sync did not select folders"
)
harness.assert(
    request_count(requests, "imap", "UID SEARCH") >= 1,
    "delta sync did not check server UIDs"
)
-- Strict on purpose: with imap-small.toml's CONDSTORE-ish state, a true
-- no-op delta need not issue UID FETCH at all. saehrimnir currently logs
-- only "UID FETCH" without item detail, so this can't yet distinguish
-- BODY.PEEK[] from FLAGS. If a deliberate flag-reconciliation pass lands,
-- extend saehrimnir's log entry and forbid body fetches only.
harness.assert_eq(
    request_count(requests, "imap", "UID FETCH"),
    0,
    "steady-state delta unexpectedly fetched message bodies or flags"
)

local after_delta, after_delta_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(after_delta_err == nil, "TestQueryDbState after delta sync failed")
harness.assert_eq(after_delta.message_count, 2, "delta message count")
harness.assert_eq(after_delta.unread_message_count, 1, "delta unread count")
harness.assert_eq(after_delta.thread_count, after_initial.thread_count, "delta thread count")
harness.assert_eq(after_delta.label_count, after_initial.label_count, "delta label count")
local delta_account = account_by_id(after_delta, account.account_id)
harness.assert(delta_account ~= nil, "account missing after delta sync")
harness.assert(delta_account.initial_sync_completed, "delta cleared initial sync flag")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
