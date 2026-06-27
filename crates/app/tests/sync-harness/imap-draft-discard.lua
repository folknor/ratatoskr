-- description: IMAP draft discard removes the draft from the server Drafts folder
-- expected: pass
-- fixture: send-small.toml
-- protocol: imap
-- ceiling: 120s
--
-- Draft-discard round-trip gate (B5-GATES). The send-small fixture carries a
-- server-side draft (draft-001, "$draft" in the Drafts mailbox). After an
-- initial sync the draft lands locally; the harness-only test.discard_draft
-- trigger resolves the draft's bifrost ObjectId and drives
-- engine.draft_discard (the remote leg of actions::delete_draft). A follow-up
-- resync must then show the draft GONE from the server - a real round-trip,
-- not just a local delete.

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

local dir = harness.data_dir("sync_imap_draft_discard")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-imap-draft@example.test",
    display_name = "Sync IMAP Draft",
    account_name = "Sync IMAP Draft",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- The fixture draft must have synced into the local Drafts view.
local before, before_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(before_err == nil, "TestQueryDbState (before) failed")
local draft = message_by_subject(before.messages, "Draft subject")
harness.assert(draft ~= nil, "fixture draft did not sync into the account")

local draft_thread, draft_thread_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = draft.thread_id,
})
harness.assert(draft_thread_err == nil, "TestThreadRead (before) failed")
harness.assert(has_label(draft_thread.label_ids, "DRAFT"), "synced draft is not under DRAFT")

-- Discard the draft server-side via the engine.
local discard, discard_err = client:request("TestDiscardDraft", {
    account_id = account.account_id,
    thread_id = draft.thread_id,
})
if discard_err ~= nil then
    harness.assert(false, "TestDiscardDraft failed: " .. tostring(discard_err.detail))
end
harness.assert(discard.discarded, "draft discard did not report success")

-- Round-trip: resync and assert the draft is gone from the server.
local resync, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(resync_err == nil, "post-discard resync failed")
harness.assert_eq(resync.result, "completed", resync.error or "post-discard resync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 50,
})
harness.assert(after_err == nil, "TestQueryDbState (after) failed")
harness.assert(
    message_by_subject(after.messages, "Draft subject") == nil,
    "draft still present after server-side discard + resync"
)

local gone, gone_err = client:request("TestThreadRead", {
    account_id = account.account_id,
    thread_id = draft.thread_id,
})
harness.assert(gone_err == nil, "TestThreadRead (after) failed")
harness.assert(not gone.exists, "draft thread still exists after discard + resync")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
