fixture({
  name = "jmap-changes-budget-exhaustion",
})

oauth({
  enforce = true,
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
  thread_id = "thread-budget",
  mailbox_ids = {"mbx-inbox"},
  from = "alice@example.com",
  to = {"bob@example.com"},
  subject = "Budget baseline",
  received_at = "2026-01-15T10:00:00Z",
  message_id = {"<budget-001@example.com>"},
  body_text = "Baseline body.",
})

-- Load-bearing TRIGGER, distinct from the budget exhaustion. This single
-- accountNotFound on the third Email/changes call (the first re-drive after the
-- initial sync's calls 1-2) is what makes the engine treat the JMAP session as
-- stale and enter its account-restart / reopen path. Without it the resident
-- kick just re-drives changes against the live session and never reopens, so
-- the companion `test/jmap/fail-open` budget (armed in jmap-pause-resume.lua)
-- would never be touched and no pause would occur. It is NOT redundant with the
-- fail-open count: it returns success again from call 4 onward, so on its own a
-- reopen would succeed and the account would recover - the fail-open arming is
-- what then forces every reopen to fail until the 3-attempt budget exhausts.
on("jmap", "Email/changes", function(req)
  if req.call_index == 3 then
    return {
      status = "accountNotFound",
      message = "forced account restart for retry-budget exhaustion",
    }
  end
end)
