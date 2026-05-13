-- description: JMAP calendar OAuth TOML fixture loads through saehrimnir
-- expected: pass
-- fixture: jmap-calendar-oauth.toml
-- protocol: jmap
-- ceiling: 60s

local endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local snapshot = harness.snapshot_state(endpoint)
harness.assert_eq(snapshot.name, "jmap-calendar-oauth", "fixture name")
harness.assert(#snapshot.mailboxes >= 1, "fixture has no mailboxes")
harness.assert(#snapshot.events >= 2, "fixture has too few events")

harness.write_summary({
    correct = 1,
    fixture = snapshot.name,
    mailbox_count = #snapshot.mailboxes,
    event_count = #snapshot.events,
})
