-- description: Graph initial sync imports fixture mail, folders, labels, and attachments
-- expected: pass
-- fixture: graph-initial.toml
-- protocol: graph
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

local function attachment_by_filename(attachments, filename)
    for _, attachment in ipairs(attachments) do
        if attachment.filename == filename then
            return attachment
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_graph_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-graph-initial@example.test",
    display_name = "Sync Graph",
    account_name = "Sync Graph",
    provider = "graph",
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
harness.assert_eq(state.message_count, 1, "message count")
harness.assert_eq(state.attachment_count, 1, "attachment count")
harness.assert(state.thread_count >= 1, "thread count")

local synced_account = account_by_id(state, account.account_id)
harness.assert(synced_account ~= nil, "account missing after sync")
harness.assert(synced_account.initial_sync_completed, "initial sync did not mark account completed")

local inbox = folder_by_id(state.folders, "INBOX")
harness.assert(inbox ~= nil, "missing INBOX folder")
harness.assert_eq(inbox.name, "INBOX", "INBOX folder name")

local message = message_by_subject(state.messages, "Graph initial")
harness.assert(message ~= nil, "missing Graph initial message")
harness.assert_eq(message.from_address, "alice@example.com", "message from_address")

local attachment = attachment_by_filename(state.attachments, "sample.txt")
harness.assert(attachment ~= nil, "missing sample.txt attachment")
harness.assert_eq(attachment.mime_type, "text/plain", "attachment mime type")
harness.assert((attachment.size or 0) > 0, "attachment size")

local work = label_by_id(state.labels, "cat:Work")
harness.assert(work ~= nil, "missing cat:Work")
harness.assert_eq(work.name, "Work", "cat:Work display name")

local urgent = label_by_id(state.labels, "cat:Urgent")
harness.assert(urgent ~= nil, "missing cat:Urgent")
harness.assert_eq(urgent.name, "Urgent", "cat:Urgent display name")

harness.assert(label_by_id(state.labels, "importance:high") ~= nil, "missing importance:high")
harness.assert(label_by_id(state.labels, "importance:low") ~= nil, "missing importance:low")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local folder_requests =
    harness.request_count(requests, "graph", "GET /v1.0/me/mailFolders")
local message_requests =
    harness.request_count_prefix(requests, "graph", "GET /v1.0/me/mailFolders/")
local master_category_requests =
    harness.request_count(requests, "graph", "GET /v1.0/me/outlook/masterCategories")
harness.assert(folder_requests >= 1, "Graph sync did not fetch folders")
harness.assert(message_requests >= 1, "Graph sync did not fetch mail")
harness.assert(master_category_requests >= 1, "Graph sync did not fetch master categories")

harness.write_summary({
    correct = 1,
    message_count = state.message_count,
    thread_count = state.thread_count,
    folder_count = state.folder_count,
    label_count = state.label_count,
    attachment_count = state.attachment_count,
    provider_requests = #requests,
    graph_folder_requests = folder_requests,
    graph_mail_requests = message_requests,
    graph_master_category_requests = master_category_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
