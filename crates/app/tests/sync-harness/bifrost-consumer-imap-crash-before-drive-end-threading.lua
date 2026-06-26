-- description: IMAP deferred ack re-drives messages after crash before drive-end threading
-- expected: pass
-- ceiling: 90s

local function read_thread(client, account_id, thread_id, label)
    local thread, err = client:request("TestThreadRead", {
        account_id = account_id,
        thread_id = thread_id,
    })
    harness.assert(err == nil, label .. " TestThreadRead failed")
    return thread
end

local function has_value(values, expected)
    for _, value in ipairs(values) do
        if value == expected then
            return true
        end
    end
    return false
end

local function message_by_id(state, message_id)
    for _, message in ipairs(state.messages) do
        if message.id == message_id then
            return message
        end
    end
    return nil
end

local dir = harness.data_dir("bifrost_consumer_imap_crash_before_drive_end_threading")
local thread_id = "imap-crash-drive-thread"
local batch_messages = {
    {
        id = "imap-crash-drive-m1",
        thread_id = thread_id,
        subject = "IMAP drive-end crash",
        from_addr = "imap-crash@example.test",
        to_addrs = { "me@example.test" },
        folder_ids = { "INBOX" },
        label_ids = {},
        keywords = {},
        raw_body = "imap crash body",
    },
}

local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "spawn A failed")

local ready_a, ready_err_a = client_a:request("BootReady")
harness.assert(ready_err_a == nil, "boot.ready A failed")
harness.assert(ready_a.ready, "boot.ready A returned ready=false")

local seeded, seed_err = client_a:request("TestSeedAccount", {
    email = "imap-crash-drive@example.test",
    provider = "imap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")
local account_id = seeded.account_id

local armed, arm_err = client_a:request("test.bifrost_arm_hook", {
    account_id = account_id,
    hook = { kind = "crash_before_drive_end_threading" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "hook was not armed")

local attach_a, attach_err_a = client_a:request("test.bifrost_attach", {
    account_id = account_id,
    provider_kind = "imap",
    detach_on_complete = false,
})
harness.assert(attach_err_a == nil, "test.bifrost_attach A failed")

local injected_a, inject_err_a = client_a:request("test.bifrost_inject_batch", {
    account_id = account_id,
    session_id = attach_a.session_id,
    scope = "folder:INBOX",
    checkpoint = { 91 },
    messages = batch_messages,
})
harness.assert(inject_err_a == nil, "test.bifrost_inject_batch A failed")
harness.assert(injected_a.acked, "inject request should represent the intended checkpoint")

local probe_a, probe_err_a = client_a:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "folder:INBOX",
    searchable_message_id = "imap-crash-drive-m1",
})
harness.assert(probe_err_a == nil, "test.bifrost_probe A failed")
harness.assert(probe_a.durable_cursor == nil, "drive-end crash must withhold cursor")
harness.assert_eq(probe_a.is_searchable, true, "message should persist before drive-end crash")

local thread_a = read_thread(client_a, account_id, thread_id, "pre-reboot")
harness.assert(
    not has_value(thread_a.label_ids, "INBOX"),
    "thread folder rollup should not flush before drive-end threading"
)

local ok_a, shutdown_err_a = client_a:shutdown()
harness.assert(ok_a, "shutdown A failed")
harness.assert(shutdown_err_a == nil, "shutdown A returned error")
client_a:drop()

local client_b, err_b = harness.spawn(dir)
harness.assert(err_b == nil, "spawn B failed")

local ready_b, ready_err_b = client_b:request("BootReady")
harness.assert(ready_err_b == nil, "boot.ready B failed")
harness.assert(ready_b.ready, "boot.ready B returned ready=false")

local attach_b, attach_err_b = client_b:request("test.bifrost_attach", {
    account_id = account_id,
    provider_kind = "imap",
    detach_on_complete = false,
})
harness.assert(attach_err_b == nil, "test.bifrost_attach B failed")

local injected_b, inject_err_b = client_b:request("test.bifrost_inject_batch", {
    account_id = account_id,
    session_id = attach_b.session_id,
    scope = "folder:INBOX",
    checkpoint = { 91 },
    messages = batch_messages,
})
harness.assert(inject_err_b == nil, "test.bifrost_inject_batch B failed")
harness.assert(injected_b.acked, "re-inject should ack after drive-end threading")

local probe_b, probe_err_b = client_b:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "folder:INBOX",
    searchable_message_id = "imap-crash-drive-m1",
})
harness.assert(probe_err_b == nil, "test.bifrost_probe B failed")
harness.assert(probe_b.durable_cursor ~= nil, "cursor must advance after re-drive")

local state_b, state_err_b = client_b:request("TestQueryDbState", {
    account_id = account_id,
    message_limit = 10,
})
harness.assert(state_err_b == nil, "TestQueryDbState B failed")
local message_b = message_by_id(state_b, "imap-crash-drive-m1")
harness.assert(message_b ~= nil, "message missing after re-drive")
local thread_b = read_thread(client_b, account_id, message_b.thread_id, "post-reboot")
harness.assert(
    has_value(thread_b.label_ids, "INBOX"),
    "deferred ack should make un-threaded messages re-drive on restart"
)

local ok_b, shutdown_err_b = client_b:shutdown()
harness.assert(ok_b, "shutdown B failed")
harness.assert(shutdown_err_b == nil, "shutdown B returned error")
