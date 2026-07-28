-- Bulk fixture with a deliberately slow middle Email/query page.
--
-- The count must exceed several multiples of the session's advertised
-- maxObjectsInGet (saehrimnir hardcodes 500), because bifrost pages
-- Email/query at that limit. 2600 emails = 6 pages, so the call_index == 3
-- latency below lands on a genuine MIDDLE page. At the legacy 250 the whole
-- mailbox fit one page and the slow-page trigger never fired.

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
  count = 2600,
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
