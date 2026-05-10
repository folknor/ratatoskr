-- Same scenario as fixtures/jmap-attach.toml. Asserted equivalent
-- by tests/lua_fixture.rs.

fixture({
  name = "jmap-attach",
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

email({
  id = "email-001",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "With attachment",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<email-001@example.com>"},
  body_text = "See attached.",
  attachments = {
    {
      blob_id = "blob-att-001",
      name = "sample.txt",
      content_type = "text/plain",
      disposition = "attachment",
      data_path = "blobs/sample.txt",
    },
  },
})
