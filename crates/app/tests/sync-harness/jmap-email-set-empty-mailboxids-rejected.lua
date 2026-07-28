-- description: JMAP Email/set refuses emptying mailboxIds; sync state stays consistent
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s
--
-- Pins the conforming-server contract promoted at saehrimnir e040fa9: any
-- Email/set update that would leave an email in NO mailbox is refused with a
-- per-item `invalidProperties` SetError (RFC 8621 has no All Mail, so the
-- state is unrepresentable), with no partial mutation and no change-log
-- transition. This is deliberately the OPPOSITE of Gmail, whose same shape
-- is an archive. Ratatoskr's own client never emits the empty shape (bulk
-- move REPLACES membership), so the consumer-side stake is different: a mock
-- that silently manufactured mailbox-less emails would feed our sync tests
-- unrepresentable state. Here we drive the refused shapes at the mock
-- directly, then prove a resync sees NOTHING changed - and that a legal
-- move still applies and flows through to our DB (guarding against an
-- over-broad rejection).

local function jmap_call(endpoint, method, args, call_id)
    local response = harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "jmap/api"),
        body = {
            using = {
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:mail",
            },
            methodCalls = {
                { method, args, call_id or "c0" },
            },
        },
    })
    harness.assert_eq(response.methodResponses[1][1], method, method .. " response method")
    return response.methodResponses[1][2]
end

local function assert_rejected(endpoint, email_id, patch, label)
    local result = jmap_call(endpoint, "Email/set", {
        accountId = "account-1",
        update = {
            [email_id] = patch,
        },
    })
    harness.assert(
        result.notUpdated ~= nil and result.notUpdated[email_id] ~= nil,
        label .. ": expected notUpdated entry"
    )
    harness.assert_eq(
        result.notUpdated[email_id]["type"],
        "invalidProperties",
        label .. ": SetError type"
    )
    harness.assert(
        result.updated == nil or result.updated[email_id] == nil,
        label .. ": must not also report updated"
    )
end

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

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("sync_jmap_email_set_empty_mailboxids")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-empty-mbx@example.test",
    display_name = "Sync JMAP Empty Mbx",
    account_name = "Sync JMAP Empty Mbx",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local first, first_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(first_err == nil, "initial start_sync failed")
harness.assert_eq(first.result, "completed", first.error or "initial sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.message_count, 2, "initial message count")
local hello = message_by_subject(state.messages, "Hello")
harness.assert(hello ~= nil, "missing Hello")

-- All three refused shapes, against the fixture's email-001 (in mbx-inbox
-- only, so every one of these would strand it in no mailbox).
assert_rejected(jmap_endpoint, "email-001", { mailboxIds = {} }, "full-replace to empty")
assert_rejected(
    jmap_endpoint,
    "email-001",
    { mailboxIds = { ["mbx-inbox"] = false } },
    "all-false replace"
)
-- Per-key form. JSON null is not representable from Lua, so this drives the
-- RFC-equivalent `false` removal encoding; the mock's own tests pin the
-- `null` variant of the same branch.
assert_rejected(
    jmap_endpoint,
    "email-001",
    { ["mailboxIds/mbx-inbox"] = false },
    "per-key removal of the last membership"
)

-- The refusals recorded no transition: a delta sync completes and changes
-- nothing on our side.
local delta, delta_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(delta_err == nil, "post-rejection start_sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "post-rejection sync result")
local after = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert_eq(after.message_count, 2, "message count after rejected mutations")
local hello_after = read_thread(client, account.account_id, hello.thread_id, "after rejections")
harness.assert(hello_after.exists, "thread missing after rejected mutations")
harness.assert(
    has_label(hello_after.label_ids, "INBOX"),
    "thread lost INBOX membership despite the mutations being rejected"
)

-- Positive control: a LEGAL move (replace membership with the archive
-- mailbox) still applies and reaches our DB on the next sync - the refusal
-- is scoped to the empty shape, not to membership writes.
-- RFC 8621 maps a successful update id to `null`, which the Lua JSON
-- conversion drops - so success is asserted as the ABSENCE of a notUpdated
-- entry here, and positively by the membership landing in our DB below.
local moved = jmap_call(jmap_endpoint, "Email/set", {
    accountId = "account-1",
    update = {
        ["email-001"] = { mailboxIds = { ["mbx-archive"] = true } },
    },
})
harness.assert(
    moved.notUpdated == nil or moved.notUpdated["email-001"] == nil,
    "legal membership replace was refused"
)
local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-move start_sync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-move sync result")
local hello_moved = read_thread(client, account.account_id, hello.thread_id, "after legal move")
harness.assert(
    has_label(hello_moved.label_ids, "archive"),
    "legal move did not reach the local DB"
)

harness.write_summary({
    correct = 1,
    message_count = after.message_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
