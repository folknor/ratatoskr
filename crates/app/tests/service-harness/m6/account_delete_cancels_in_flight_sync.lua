-- description: account.delete cancels an in-flight sync runner before deleting the account
-- ceiling: 30s

local dir = harness.data_dir("m6_account_delete_cancels_sync")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-delete-sync@example.test",
    provider = "harness-slow-sync",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    thread_id = "m6-delete-sync-thread",
    message_id = "m6-delete-sync-message",
    subject = "delete during sync",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(before_err == nil, "TestQueryDbState before delete failed")
harness.assert_eq(before.account_count, 1, "account exists before delete")
harness.assert_eq(before.thread_count, 1, "thread exists before delete")
harness.assert_eq(before.message_count, 1, "message exists before delete")

local started, start_err = client:request("TestStartSync", {
    account_id = account.account_id,
})
harness.assert(start_err == nil, "TestStartSync failed")
harness.assert(not started.already_in_flight, "first sync start was duplicate")
harness.assert(started.run_id ~= nil, "sync run_id missing")

local duplicate, duplicate_err = client:request("TestStartSync", {
    account_id = account.account_id,
})
harness.assert(duplicate_err == nil, "duplicate TestStartSync failed")
harness.assert(duplicate.already_in_flight, "sync was not in flight")
harness.assert_eq(duplicate.run_id, started.run_id, "duplicate run_id")

local deleted, delete_err = client:request("account.delete", {
    account_id = account.account_id,
})
harness.assert(delete_err == nil, "account.delete failed")
harness.assert(deleted.search_cleaned, "search cleanup did not complete")

local marker_path = dir .. "/sync_markers/" .. account.account_id .. ".json"
harness.assert(harness.path_exists(marker_path), "sync marker not written")
local marker = harness.read_json(marker_path)
harness.assert_eq(marker.run_id, started.run_id, "sync marker run_id")
harness.assert_eq(marker.status, "cancelled", "sync marker status")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
})
harness.assert(after_err == nil, "TestQueryDbState after delete failed")
harness.assert_eq(after.account_count, 0, "account still exists after delete")
harness.assert_eq(after.label_count, 0, "labels still exist after delete")
harness.assert_eq(after.thread_count, 0, "threads still exist after delete")
harness.assert_eq(after.thread_label_count, 0, "thread labels still exist after delete")
harness.assert_eq(after.message_count, 0, "messages still exist after delete")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
