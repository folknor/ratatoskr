-- description: non-forced wipe index rebuild completes and leaves search usable
-- ceiling: 90s

local unique = "wipeindexroundtrip"

local function wait_for_rebuild_completed(queue, rebuild_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "index.rebuild_completed"
            and notification.rebuild_id == rebuild_id
        then
            return notification
        end
    end
    return nil
end

local function search(client, account_id, query)
    local result, result_err = client:request("TestSearchIndex", {
        account_id = account_id,
        query = query,
        limit = 10,
    })
    harness.assert(result_err == nil, "TestSearchIndex failed")
    return result
end

local function wait_for_result(client, account_id, query, message_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local result = search(client, account_id, query)
        for _, row in ipairs(result.results) do
            if row.message_id == message_id then
                return row
            end
        end
        harness.sleep(250)
    end
    return nil
end

local dir = harness.data_dir("extract_index_rebuild_wipe_roundtrip")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "extract-wipe-roundtrip@example.test",
    display_name = "Extract Wipe Roundtrip",
    account_name = "Extract Wipe Roundtrip",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local thread, thread_err = client:request("TestSeedThread", {
    account_id = account.account_id,
    subject = "Extract wipe rebuild roundtrip",
    label_ids = { "INBOX" },
    is_read = true,
    body_text = "The normal wipe rebuild should index " .. unique .. " from message body text.",
})
harness.assert(thread_err == nil, "TestSeedThread failed")

local rebuild, rebuild_err = client:request("IndexRebuild", {
    policy = "wipe",
    force = false,
})
harness.assert(rebuild_err == nil, "index.rebuild failed")
harness.assert(rebuild.rebuild_id ~= nil, "rebuild_id missing")

local completed = wait_for_rebuild_completed(queue, rebuild.rebuild_id, 30)
harness.assert(completed ~= nil, "wipe rebuild did not complete")

local result = wait_for_result(client, account.account_id, unique, thread.message_id, 30)
harness.assert(result ~= nil, "rebuilt index did not return seeded message")
harness.assert_eq(result.message_id, thread.message_id, "search result message")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
