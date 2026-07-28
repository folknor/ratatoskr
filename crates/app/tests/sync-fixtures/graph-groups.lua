-- Directory-group fixture for the B8-groups consumer cut.
--
-- Four groups, one of which (`grp-hidden`) is mail-disabled and must never
-- import: bifrost drops mail-disabled rows client-side, so its absence from
-- `contact_groups` is the gate on that filter surviving the cut.
--
-- Two scripted failure windows drive the reconcile gate. Both key on
-- `req.call_index`, which saehrimnir counts PER (protocol, command) tag and
-- which is ONE-BASED (`src/lua.rs` `run_dispatch`). Two arithmetic traps the
-- reader needs:
--
--   1. `/groups/{id}/members`, `/groups/{id}/transitiveMembers` and the
--      `/microsoft.graph.user` cast all alias the single
--      `list_group_members` tag (`src/graph/group_sync.rs:48-58`), so the
--      counter is shared across every one of those routes, not per route.
--      The enumeration walks a separate tag (`list_member_of`), so the two
--      windows never interfere.
--   2. The expansion count per pull is the number of IMPORTED groups, which
--      changes mid-script, AND the resident cycle-zero aux pass performs a
--      real pull of its own before the script's explicit ones.
--      `graph_groups_reconcile.lua` waits for that aux pull to land (it polls
--      the DB rather than sleeping blind), so the enumerations run in a fixed
--      order:
--
--        aux cycle-0 pull  memberOf 1   expansions 1-3 (3 groups)
--        pull 1 baseline   memberOf 2   expansions 4-6 (3 groups)
--        step `reconcile` destroys grp-sec
--        pull 2 reconcile  memberOf 3   expansions 7-8 (2 groups)
--        pull 3 transient  memberOf 4   expansions 9-10  <- expansion window
--        pull 4 denial     memberOf 5                    <- enumeration window

fixture({ name = "graph-groups", state = "grp-0" })

account({ id = "account-1", name = "owner@example.test", is_personal = true, primary = true })
account({ id = "account-2", name = "member@example.test", is_personal = false })

mailbox({ id = "inbox", name = "Inbox", role = "inbox", sort_order = 0 })

group({
    id = "grp-unified",
    display_name = "Unified",
    mail = "unified@example.test",
    mail_enabled = true,
    group_types = { "Unified" },
    members = { "account-1" },
})
group({
    id = "grp-dl",
    display_name = "Distribution",
    mail = "dl@example.test",
    mail_enabled = true,
    members = { "account-1", "account-2" },
})
group({
    id = "grp-sec",
    display_name = "Security",
    mail = "security@example.test",
    mail_enabled = true,
    security_enabled = true,
    members = { "account-1" },
})
-- Mail-disabled: bifrost drops it, so it must never reach contact_groups.
group({
    id = "grp-hidden",
    display_name = "Hidden",
    mail_enabled = false,
    members = { "account-1" },
})

-- Step 1: a remote group deletion plus a membership shrink, the two remote
-- edits the prune and member-replace paths have to reconcile against.
change({
    id = "reconcile",
    group_destroy = { "grp-sec" },
    group_update = {
        { id = "grp-dl", members = { "account-1" } },
    },
})

-- Step 2: put the second member back on grp-dl. Applied immediately before
-- the pull whose expansions are scripted to fail, so "keep the existing
-- member rows" is observable rather than vacuous: a consumer that let a
-- failed expansion through would show two members here instead of one.
--
-- Deliberately an ADD, not a removal. `/me/memberOf` enumerates the groups
-- the bearer account belongs to, so dropping `account-1` from grp-dl would
-- make the group legitimately vanish from the enumeration and be pruned -
-- testing the prune again rather than the expansion-failure path.
change({
    id = "regrow",
    group_update = {
        { id = "grp-dl", members = { "account-1", "account-2" } },
    },
})

-- Pull 3's two expansion calls fail transiently. The consumer must keep both
-- group rows AND their existing member rows (`members: None`), never wiping
-- membership because an expansion blipped.
on("graph", "list_group_members", function(req)
    if req.call_index == 9 or req.call_index == 10 then
        return { status = "serviceUnavailable", message = "scripted transient expansion failure" }
    end
end)

-- Pull 4's ENUMERATION fails with a permission-shaped envelope - the
-- unconsented-tenant case. The consumer must return an error and write
-- NOTHING: a prune here would turn a revoked consent into local data loss.
on("graph", "list_member_of", function(req)
    if req.call_index == 5 then
        return { status = "accessDenied", message = "scripted tenant consent missing" }
    end
end)
