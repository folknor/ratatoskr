-- Many-folder JMAP fixture for exercising mailbox/label import at scale.

fixture({ name = "jmap-many-folders" })

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

bulk_mailboxes({
  count = 64,
  branching = 4,
  seed = 17,
  id_prefix = "mbf",
})

email({
  id = "many-folder-marker",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Many folders marker",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<many-folder-marker@example.com>"},
  body_text = "Marker email for the many-folder fixture.",
})
