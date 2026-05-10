-- Three separate JMAP messages sharing the same Message-ID header.
-- Ratatoskr should import all rows rather than deduplicating by header.

fixture({ name = "jmap-duplicate-message-id" })

account({
  id = "account-1",
  name = "test@example.com",
})

mailbox({
  id = "mbx-inbox",
  name = "Inbox",
  role = "inbox",
  sort_order = 0,
})

mailbox({
  id = "mbx-archive",
  name = "Archive",
  role = "archive",
  sort_order = 1,
})

email({
  id = "dup-001",
  thread_id = "thread-dup-001",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Duplicate alpha",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<duplicate@example.com>"},
  body_text = "First copy of the duplicate Message-ID case.",
})

email({
  id = "dup-002",
  thread_id = "thread-dup-002",
  mailbox_ids = {"mbx-inbox"},
  from = "carol@example.com",
  to = {"bob@example.com"},
  subject = "Duplicate beta",
  received_at = "2026-01-15T10:05:00Z",
  message_id = {"<duplicate@example.com>"},
  body_text = "Second copy of the duplicate Message-ID case.",
})

email({
  id = "dup-003",
  thread_id = "thread-dup-003",
  mailbox_ids = {"mbx-archive"},
  from = "dave@example.com",
  to = {"bob@example.com"},
  subject = "Duplicate gamma",
  received_at = "2026-01-15T10:10:00Z",
  message_id = {"<duplicate@example.com>"},
  body_text = "Third copy of the duplicate Message-ID case.",
})
