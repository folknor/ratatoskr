-- description: JMAP bulk fixture steady-state delta avoids full query fallback
-- expected: pass
-- fixture: jmap-bulk.lua
-- protocol: jmap
-- ceiling: 180s

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_bulk_steady_state_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-bulk-delta@example.test",
    display_name = "Sync JMAP Bulk Delta",
    account_name = "Sync JMAP Bulk Delta",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({
    account_id = account.account_id,
}, 120)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(before_err == nil, "pre-delta TestQueryDbState failed")
harness.assert_eq(before.message_count, 10001, "pre-delta message count")

harness.clear_mock_requests(admin_endpoint)

harness.marker("SYNC_START")
local delta, delta_err = client:start_sync({
    account_id = account.account_id,
}, 60)
harness.marker("SYNC_END")
harness.assert(delta_err == nil, "delta start_sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "delta sync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(after_err == nil, "post-delta TestQueryDbState failed")
harness.assert_eq(after.message_count, 10001, "post-delta message count")
harness.assert_eq(after.thread_count, before.thread_count, "thread count drifted")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local mailbox_changes_requests = harness.request_count(requests, "jmap", "Mailbox/changes")
local email_changes_requests = harness.request_count(requests, "jmap", "Email/changes")
local email_query_requests = harness.request_count(requests, "jmap", "Email/query")
harness.assert(mailbox_changes_requests >= 1, "delta sync did not call Mailbox/changes")
harness.assert(email_changes_requests >= 1, "delta sync did not call Email/changes")
harness.assert_eq(email_query_requests, 0, "steady-state delta fell back to Email/query")

harness.write_summary({
    correct = 1,
    measured_syncs = 1,
    message_count = after.message_count,
    thread_count = after.thread_count,
    provider_requests = #requests,
    jmap_mailbox_changes_requests = mailbox_changes_requests,
    jmap_email_changes_requests = email_changes_requests,
    jmap_email_query_requests = email_query_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
