-- description: draft WAL drains before boot.ready and rotates after replay
-- ceiling: 45s

local dir = harness.data_dir("m6_draft_wal_replays_on_boot")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "initial spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "initial boot.ready failed")
harness.assert(ready.ready, "initial boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-draft-wal@example.test",
    display_name = "Draft WAL",
    account_name = "Draft WAL",
    provider = "imap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "initial shutdown failed")
harness.assert(shutdown_err == nil, "initial shutdown returned error")
client:drop()

local wal = '{"epoch_ms":1700000000000,"params":{'
    .. '"id":"m6-draft-wal",'
    .. '"account_id":"' .. account.account_id .. '",'
    .. '"to_addresses":"to@example.test",'
    .. '"cc_addresses":"cc@example.test",'
    .. '"bcc_addresses":"bcc@example.test",'
    .. '"subject":"WAL replay subject",'
    .. '"body_html":"<p>WAL replay body</p>",'
    .. '"reply_to_message_id":"reply-msg",'
    .. '"thread_id":"thread-wal",'
    .. '"from_email":"m6-draft-wal@example.test",'
    .. '"signature_id":"sig-wal",'
    .. '"remote_draft_id":"remote-wal",'
    .. '"attachments":"[]",'
    .. '"signature_separator_index":17}}\n'
    .. 'this is a partial trailing line from a killed writer\n'
harness.write_text(dir .. "/drafts.wal", wal)
harness.assert(harness.path_exists(dir .. "/drafts.wal"), "draft WAL was not written")

local second, second_err = harness.spawn(dir)
harness.assert(second_err == nil, "second spawn failed")

local second_ready, second_ready_err = second:request("BootReady")
harness.assert(second_ready_err == nil, "second boot.ready failed")
harness.assert(second_ready.ready, "second boot.ready returned ready=false")

harness.assert(
    not harness.path_exists(dir .. "/drafts.wal"),
    "active draft WAL was not rotated"
)
harness.assert(
    harness.dir_has_prefix(dir, "drafts.wal.replayed."),
    "replayed draft WAL file missing"
)

local state, state_err = second:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.local_draft_count, 1, "local draft count")

local draft = state.local_drafts[1]
harness.assert(draft ~= nil, "local draft row missing")
harness.assert_eq(draft.id, "m6-draft-wal", "draft id")
harness.assert_eq(draft.account_id, account.account_id, "draft account")
harness.assert_eq(draft.to_addresses, "to@example.test", "draft to")
harness.assert_eq(draft.cc_addresses, "cc@example.test", "draft cc")
harness.assert_eq(draft.bcc_addresses, "bcc@example.test", "draft bcc")
harness.assert_eq(draft.subject, "WAL replay subject", "draft subject")
harness.assert_eq(draft.body_html, "<p>WAL replay body</p>", "draft body")
harness.assert_eq(draft.reply_to_message_id, "reply-msg", "draft reply-to")
harness.assert_eq(draft.thread_id, "thread-wal", "draft thread")
harness.assert_eq(draft.from_email, "m6-draft-wal@example.test", "draft from")
harness.assert_eq(draft.signature_id, "sig-wal", "draft signature id")
harness.assert_eq(draft.remote_draft_id, "remote-wal", "remote draft id")
harness.assert_eq(draft.attachments, "[]", "draft attachments")
harness.assert_eq(draft.signature_separator_index, 17, "signature separator")
harness.assert_eq(draft.sync_status, "pending", "draft status")

local second_ok, second_shutdown_err = second:shutdown()
harness.assert(second_ok, "second shutdown failed")
harness.assert(second_shutdown_err == nil, "second shutdown returned error")
