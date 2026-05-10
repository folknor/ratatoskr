-- Bulk-generated fixture demonstrating `bulk_emails`. 10k emails in
-- the inbox plus a handful of hand-authored ones, all backed by the
-- same template pools dev-seed uses (sans the locales / attachments
-- / threading bits we don't yet need).

fixture({ name = "jmap-bulk" })

account({
  id = "account-1",
  name = "test@example.com",
})

mailbox({
  id = "mbx-inbox",
  name = "Inbox",
  role = "inbox",
})

mailbox({
  id = "mbx-archive",
  name = "Archive",
  role = "archive",
})

bulk_emails({
  count = 10000,
  mailbox = "mbx-inbox",
  seed = 42,
  start_at = "2026-01-01T00:00:00Z",
  interval_seconds = 60,
})

-- Hand-authored entries can sit alongside; just ensure their ids do
-- not collide with the bulk prefix.
email({
  id = "marker-001",
  mailbox_ids = {"mbx-archive"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Hello",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<marker-001@example.com>"},
  body_text = "Hand-authored marker email.",
})
