-- description: Graph directory-group reconcile prunes, replaces members, and never destroys state on failure
-- expected: pass
-- fixture: graph-groups.lua
-- protocol: graph
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")

local client, spawn_err = harness.spawn(harness.data_dir("graph_groups_reconcile"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot ready")
local account, seed_err = client:request("TestSeedAccount", {
    email = "owner@example.test", provider = "graph",
})
harness.assert(seed_err == nil, "seed graph account")

local function groups_by_server_id()
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account.account_id, contact_group_limit = 20,
    })
    harness.assert(state_err == nil, "read group state")
    local by_id = {}
    for _, group in ipairs(state.contact_groups) do by_id[group.server_id] = group end
    return by_id, #state.contact_groups
end

-- Order-independent flattening of the whole group + member snapshot, so the
-- two no-data-loss assertions compare state rather than row order.
local function snapshot()
    local by_id, count = groups_by_server_id()
    local keys = {}
    for server_id in pairs(by_id) do keys[#keys + 1] = server_id end
    table.sort(keys)
    local flat = {}
    for _, server_id in ipairs(keys) do
        local group = by_id[server_id]
        local members = {}
        for _, email in ipairs(group.member_emails) do members[#members + 1] = email end
        table.sort(members)
        flat[#flat + 1] = table.concat({
            group.id, group.name, group.source, group.group_type or "",
            group.email or "", table.concat(members, ","),
        }, "|")
    end
    return table.concat(flat, ";"), count
end

-- Attaching the resident account starts the five-second aux timer, and its
-- cycle-zero pass runs a real group pull. The fixture's call_index windows
-- are numbered from that pull, so wait for it to LAND (poll the DB, do not
-- sleep blind) before issuing any explicit pull. The next aux pass is 300s
-- out, well past the ceiling.
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil, "initial sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "initial sync result")
local aux_landed = false
for _ = 1, 24 do
    harness.sleep(1000)
    local _, count = groups_by_server_id()
    if count == 3 then
        aux_landed = true
        break
    end
end
harness.assert(aux_landed, "cycle-zero aux group pull did not land")

-- Pull 1: the baseline snapshot. Expansions consume call_index 4-6.
local _, first_err = client:request("TestGroupPull", { account_id = account.account_id })
harness.assert(first_err == nil, "baseline pull")
local baseline, baseline_count = groups_by_server_id()
harness.assert_eq(baseline_count, 3, "three groups imported")
harness.assert(baseline["grp-hidden"] == nil, "mail-disabled group must never import")
harness.assert_eq(#baseline["grp-dl"].member_emails, 2, "DL starts with two members")

-- Step 1: remote deletion of grp-sec plus a membership shrink on grp-dl.
local step = harness.http_json({
    method = "POST", url = harness.join_url(admin, "test/fixture/step"),
    body = { expect = "reconcile" },
})
harness.assert(step.ok, "apply reconcile fixture step")

-- Pull 2: the reconcile. Expansions consume call_index 7-8.
local _, second_err = client:request("TestGroupPull", { account_id = account.account_id })
harness.assert(second_err == nil, "reconcile pull")
local after_step, after_step_count = groups_by_server_id()
harness.assert_eq(after_step_count, 2, "destroyed group was not pruned")
harness.assert(after_step["grp-sec"] == nil, "grp-sec survived the prune")
harness.assert_eq(#after_step["grp-dl"].member_emails, 1, "DL member list was not replaced")
harness.assert_eq(after_step["grp-dl"].member_emails[1], "owner@example.test", "surviving member")
local pruned_snapshot = snapshot()

-- Step 2: restore grp-dl's second member remotely, immediately before the
-- pull whose expansions are scripted to fail. The remote truth is now two
-- members; the consumer must NOT adopt it, because it never successfully
-- read it. Without this step the "members intact" assertion would pass
-- vacuously - the remote and local sets would already agree.
local regrow = harness.http_json({
    method = "POST", url = harness.join_url(admin, "test/fixture/step"),
    body = { expect = "regrow" },
})
harness.assert(regrow.ok, "apply regrow fixture step")

-- Pull 3: both expansions fail transiently (call_index 9-10). Every group row
-- must survive with its pull-2 member rows INTACT - a transient expansion
-- failure must never wipe membership, and must not fail the pull.
local _, third_err = client:request("TestGroupPull", { account_id = account.account_id })
harness.assert(third_err == nil, "transient expansion failure must not fail the pull")
local after_transient, after_transient_count = snapshot()
harness.assert_eq(after_transient_count, 2, "group lost to a transient expansion failure")
harness.assert_eq(after_transient, pruned_snapshot, "members lost to a transient expansion failure")
local transient_groups = groups_by_server_id()
harness.assert_eq(#transient_groups["grp-dl"].member_emails, 1,
    "failed expansion must not adopt the remote member set it never read")

-- Pull 4: the ENUMERATION fails with a permission envelope (call_index 5 on
-- the list_member_of tag). This is the only gate on O4: the request must
-- error and the stored snapshot must be identical to pull 3's. An
-- implementation that pruned on a permission failure passes every other
-- assertion in this file.
local _, fourth_err = client:request("TestGroupPull", { account_id = account.account_id })
harness.assert(fourth_err ~= nil, "enumeration permission failure must surface as an error")
local after_denied = snapshot()
harness.assert_eq(after_denied, after_transient, "permission failure destroyed local group state")

-- A non-directory provider is a capability no-op, not a delete-all. Both
-- report zero groups; only `supported` tells them apart.
local jmap_account, jmap_seed_err = client:request("TestSeedAccount", {
    email = "member@example.test", provider = "jmap",
})
harness.assert(jmap_seed_err == nil, "seed jmap account")
local _, save_err = client:request("contacts.group_save", {
    id = "user-group-1", name = "Book Club",
    member_emails = { "reader@example.test" },
})
harness.assert(save_err == nil, "seed local user group")
local jmap_pull, jmap_pull_err = client:request("TestGroupPull", {
    account_id = jmap_account.account_id,
})
harness.assert(jmap_pull_err == nil, "jmap group pull")
harness.assert_eq(jmap_pull.supported, false, "jmap must report the surface unsupported")
harness.assert_eq(jmap_pull.groups, 0, "jmap must import no groups")

local all_groups, all_err = client:request("TestQueryDbState", { contact_group_limit = 20 })
harness.assert(all_err == nil, "read all groups")
local user_group
for _, group in ipairs(all_groups.contact_groups) do
    if group.id == "user-group-1" then user_group = group end
end
harness.assert(user_group ~= nil, "user-authored group was destroyed by the group pull")
harness.assert_eq(user_group.source, "user", "user group source")

harness.write_summary({ correct = 1, group_count = after_transient_count })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown")
