-- JMAP raw body fixture with malformed multipart-ish bytes. Structured
-- metadata stays valid while the body path receives anomalous content.

fixture({ name = "jmap-malformed-mime" })

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

email({
  id = "malformed-001",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Malformed multipart boundary",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<malformed-001@example.com>"},
  body_text = "Fallback structured body.",
  body_raw_bytes = "From: alice@example.com\r\nSubject: malformed\r\nContent-Type: multipart/mixed; boundary=\"X\"\r\n\r\n--X-but-no-real-boundary\r\nbroken body\r\n",
})
