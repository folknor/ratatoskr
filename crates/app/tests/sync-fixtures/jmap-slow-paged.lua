-- Bulk fixture with a deliberately slow middle Email/query page.

fixture({ name = "jmap-slow-paged" })

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

bulk_emails({
  count = 250,
  mailbox = "mbx-inbox",
  seed = 71,
  start_at = "2026-01-01T00:00:00Z",
  interval_seconds = 60,
  id_prefix = "slow",
})

on("jmap", "Email/query", function(req)
  if req.call_index == 3 then
    wait(250)
  end
  return nil
end)
