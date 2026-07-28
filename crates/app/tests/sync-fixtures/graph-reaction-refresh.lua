-- The Graph reaction refresh only considers messages that already carry an
-- `exchange_native` reaction row OR arrived within the last 14 days (the
-- legacy seeded-or-recent LIMIT 60 candidate query, ported verbatim into
-- `service::bifrost::aux::graph`). A `received_at` of "now minus more than a
-- fortnight" silently drops the message out of the candidate set and the gate
-- stops exercising the refresh while still reporting green on its other
-- assertions - which is exactly what a fixed 2026-01-15 date did.
--
-- The fixture sandbox has no clock (`os` is not exposed), so pin a date that
-- is unconditionally inside the window instead of one that ages out of it.
-- Nothing else in this fixture is date-sensitive.
local received_at = "2099-01-01T00:00:00Z"

fixture({ name = "graph-reaction-refresh" })
account({ id = "account-1", name = "test@example.com" })
mailbox({ id = "mbx-inbox", name = "Inbox", role = "inbox", sort_order = 0 })
category({ id = "cat-work", display_name = "Work", color = "preset0" })
email({
    id = "email-001",
    thread_id = "thread-graph-reactions",
    mailbox_ids = { "mbx-inbox" },
    from = "alice@example.com",
    to = { "test@example.com" },
    subject = "Reaction refresh",
    received_at = received_at,
    message_id = { "<graph-reaction-001@example.com>" },
    body_text = "Reaction refresh body.",
    reaction_type = "like",
    reaction_count = 3,
})

-- `email_reaction` is an ARRAY of entries, not a single entry: passing one
-- bare table yields a length of zero and the step applies NOTHING while still
-- reporting ok. `clear = true` is also the only way to remove a reaction -
-- `reaction_type = nil` is indistinguishable from an absent key in Lua, and an
-- absent field means "leave this slot alone", so the nil spelling was a no-op
-- twice over.
--
-- Clearing wipes BOTH slots server-side, which is the stronger shape for what
-- the gate proves: the consumer must delete the owner row (it has a
-- delete-on-absent branch) and must KEEP the now-stale `__count__` row (it has
-- no such branch - B15 section 2.8 bug 1).
change({
    id = "clear-owner-reaction",
    email_reaction = { { id = "email-001", clear = true } },
})
