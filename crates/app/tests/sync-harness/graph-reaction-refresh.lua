-- description: Graph Exchange reactions refresh through the resident auxiliary cadence
-- expected: pass
-- fixture: graph-reaction-refresh.lua
-- protocol: graph
-- ceiling: 120s
--
-- The reaction refresh reads the extended-property pair through the NESTED
-- property collection (`GET .../messages/{id}/singleValueExtendedProperties`
-- `?$filter=...`), chunked into a `$batch` - not the `$expand` clause on the
-- message. saehrimnir served only the `$expand` shape until df9a300, so every
-- batch item returned 501 and the read landed wholly in `BatchOutcome::failed`,
-- which (correctly, per B15 spec 5.2) writes no rows: a failed item must never
-- be classified as "no reaction", or a transient Graph error would wipe cached
-- reactions. Both shapes now project off one source, and an unstaged message
-- answers a clean 200 with an empty collection rather than an error.

local client, spawn_err = harness.spawn(harness.data_dir("graph_reaction_refresh"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot ready")

local account, seed_err = client:request("TestSeedAccount", {
    email = "test@example.com", provider = "graph",
})
harness.assert(seed_err == nil, "seed graph account")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil, "initial sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "initial sync result")

-- Drive the DELTA branch synchronously. `initial_sync_completed = true` skips
-- the initial branch (which imports master categories and returns without
-- touching reactions); seeding the cadence counter at 4 means the pass's own
-- production increment lands on 5, the reaction-refresh cycle. The production
-- driver is a 5s-then-300s wall-clock timer that sync kicks never invoke, so
-- without this affordance the first refresh is ~25 minutes after attach.
local _, aux_err = client:request("TestGraphAuxPass", {
    account_id = account.account_id,
    cycle = 4,
    initial_sync_completed = true,
})
harness.assert(aux_err == nil, "reaction refresh aux pass")

local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id })
harness.assert(state_err == nil, "read reaction state")
local owner, count = nil, nil
for _, reaction in ipairs(state.message_reactions) do
    if reaction.reactor_email == account.email then owner = reaction end
    if reaction.reactor_email == "__count__" then count = reaction end
end
harness.assert(owner ~= nil, "owner reaction row missing")
harness.assert_eq(owner.reaction_type, "like", "owner reaction type")
harness.assert(count ~= nil, "reaction count row missing")
harness.assert_eq(count.reaction_type, "3", "reaction count")

harness.assert_eq(account.email, "test@example.com", "owner row is keyed on accounts.email")

-- B15 section 2.8 bug 1, pinned rather than fixed: the owner row has a
-- delete-on-absent branch but the `__count__` row does NOT, so a message whose
-- reactions are cleared server-side keeps its count row forever. Asserting the
-- count row SURVIVES is what makes the omission read as deliberate - and makes
-- the eventual data-correctness fix show up as a gate change, not silent drift.
-- saehrimnir serves ONE admin surface, on the JMAP listener, whatever protocol
-- the fixture speaks - hence the JMAP endpoint var in a Graph gate.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
-- `expect` names the change-script step and 409s on a cursor mismatch, so an
-- out-of-phase step fails loudly instead of silently applying the wrong one.
local response = harness.http_json({
    method = "POST",
    url = harness.join_url(admin_endpoint, "test/fixture/step"),
    body = { expect = "clear-owner-reaction" },
})
harness.assert(response.ok, "stage owner-reaction clear")

local _, clear_err = client:request("TestGraphAuxPass", {
    account_id = account.account_id,
    cycle = 9,
    initial_sync_completed = true,
})
harness.assert(clear_err == nil, "clear reaction aux pass")
local after, after_err = client:request("TestQueryDbState", { account_id = account.account_id })
harness.assert(after_err == nil, "read cleared reaction state")
local stale_count = nil
for _, reaction in ipairs(after.message_reactions) do
    harness.assert(reaction.reactor_email ~= account.email, "owner reaction was not removed")
    if reaction.reactor_email == "__count__" then stale_count = reaction end
end
harness.assert(stale_count ~= nil, "stale __count__ row vanished - section 2.8 bug 1 was fixed")

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown")
