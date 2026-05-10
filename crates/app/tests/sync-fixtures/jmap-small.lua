-- Same scenario as fixtures/jmap-small.toml. Authored as a Lua
-- script via dellingr so we exercise the same code path the dynamic
-- features (reactive callbacks, etc.) will use.
--
-- Note: dellingr does not support unparenthesized function calls,
-- so builders are written `mailbox({...})` rather than
-- `mailbox {...}`.

fixture({
  name = "jmap-small",
})

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
  id = "email-001",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Hello",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<email-001@example.com>"},
  body_text = "First message body.",
})

email({
  id = "email-002",
  mailbox_ids = {"mbx-inbox"},
  from = "carol@example.com",
  to = {"bob@example.com"},
  subject = "Re: Hello",
  received_at = "2026-01-15T11:00:00Z",
  message_id = {"<email-002@example.com>"},
  in_reply_to = {"<email-001@example.com>"},
  references = {"<email-001@example.com>"},
  body_text = "Reply body.",
})
