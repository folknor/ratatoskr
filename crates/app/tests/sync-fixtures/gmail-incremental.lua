-- Gmail incremental-sync scenario: a steady-state baseline plus a
-- four-step change script that exercises the Gmail `history.list`
-- record types ratatoskr's `changes_from_history` consumes:
--   1. a new message               -> messagesAdded
--   2. a deleted message           -> messagesDeleted
--   3. a label add + remove        -> labelsAdded + labelsRemoved
--   4. a label change on ONE message of a multi-message thread,
--      proving the record scopes to that message id, not the thread.
--
-- Each step lands as one Transition in the per-account change_log;
-- the Gmail listener projects the log into history records keyed by
-- the monotonic `historyId` (change-log counter + 1). Drives
-- `tests/step.rs::gmail_history_*`.

fixture({ name = "gmail-incremental", state = "gh-0" })
account({ id = "account-1", name = "test@example.com" })

mailbox({ id = "mb-inbox", name = "Inbox", role = "inbox", sort_order = 0 })
mailbox({ id = "mb-archive", name = "Archive", role = "archive", sort_order = 1 })

-- email-001: plain read inbox message; deleted in step 2.
email({
    id = "email-001",
    thread_id = "thread-001",
    mailbox_ids = { "mb-inbox" },
    keywords = { "$seen" },
    received_at = "2026-01-15T10:00:00Z",
    from = { name = "Alice", email = "alice@example.com" },
    to = { "test@example.com" },
    subject = "Welcome",
    body_text = "Hello there.",
    message_id = { "<001@example.com>" },
})

-- email-002: carries a user label `old`; step 3 swaps it for `new`.
email({
    id = "email-002",
    thread_id = "thread-002",
    mailbox_ids = { "mb-inbox" },
    keywords = { "$seen", "old" },
    received_at = "2026-01-15T11:00:00Z",
    from = "bob@example.com",
    to = { "test@example.com" },
    subject = "Status update",
    body_text = "Things are progressing.",
    message_id = { "<002@example.com>" },
})

-- email-003a / email-003b: a two-message thread. Step 4 stars only
-- email-003a, so the history record must name email-003a alone.
email({
    id = "email-003a",
    thread_id = "thread-003",
    mailbox_ids = { "mb-inbox" },
    keywords = { "$seen" },
    received_at = "2026-01-15T12:00:00Z",
    from = "carol@example.com",
    to = { "test@example.com" },
    subject = "Lunch?",
    body_text = "Free at 12:30?",
    message_id = { "<003a@example.com>" },
})

email({
    id = "email-003b",
    thread_id = "thread-003",
    mailbox_ids = { "mb-inbox" },
    keywords = { "$seen" },
    received_at = "2026-01-15T12:30:00Z",
    from = "test@example.com",
    to = { "carol@example.com" },
    subject = "Re: Lunch?",
    body_text = "Sounds good.",
    message_id = { "<003b@example.com>" },
})

-- Step 1: a new message arrives in the inbox -> messagesAdded.
change({
    id = "new",
    email_create = {
        {
            id = "email-100",
            thread_id = "thread-100",
            mailbox_ids = { "mb-inbox" },
            received_at = "2026-01-15T13:00:00Z",
            from = "dave@example.com",
            to = { "test@example.com" },
            subject = "New thing",
            body_text = "Just landed.",
            message_id = { "<100@example.com>" },
        },
    },
})

-- Step 2: a message is deleted -> messagesDeleted.
change({
    id = "delete",
    email_destroy = { "email-001" },
})

-- Step 3: swap the user label on email-002 (drop `old`, add `new`)
-- -> labelsRemoved [Label_old] + labelsAdded [Label_new].
change({
    id = "label",
    email_update = {
        { id = "email-002", keywords = { "$seen", "new" } },
    },
})

-- Step 4: star one message of a two-message thread
-- -> labelsAdded [STARRED] scoped to email-003a only.
change({
    id = "thread-label",
    email_update = {
        { id = "email-003a", keywords = { "$seen", "$flagged" } },
    },
})
