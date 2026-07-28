-- description: provider-agnostic Graph directory groups pull
-- expected: pass
-- fixture: graph-groups.lua
-- protocol: graph
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin ~= nil, "saehrimnir admin endpoint missing")
local client, spawn_err = harness.spawn(harness.data_dir("graph_groups_pull"))
harness.assert(spawn_err == nil, "spawn failed")
local _, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot ready")
local account, seed_err = client:request("TestSeedAccount", { email = "owner@example.test", provider = "graph" })
harness.assert(seed_err == nil, "seed")
-- Fence out the resident cycle-zero aux pass. This benchmark measures only
-- the explicit TestGroupPull request, not timer scheduling.
harness.sleep(6000)
local quiet = #harness.mock_requests(admin)
for _ = 1, 20 do
  harness.sleep(500)
  local now = #harness.mock_requests(admin)
  if now == quiet then break end
  quiet = now
end
harness.clear_mock_requests(admin)
local pull, pull_err = client:request("TestGroupPull", { account_id = account.account_id })
harness.assert(pull_err == nil and pull.supported and pull.groups == 3, "group pull")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, contact_group_limit = 20 })
harness.assert(state_err == nil and #state.contact_groups == 3, "three imported groups")
local by_id = {}
for _, group in ipairs(state.contact_groups) do by_id[group.server_id] = group end
harness.assert(by_id["grp-hidden"] == nil, "mail-disabled group must never import")
harness.assert_eq(by_id["grp-unified"].group_type, "m365", "unified type")
harness.assert_eq(by_id["grp-dl"].group_type, "distribution_list", "DL type")
harness.assert_eq(by_id["grp-sec"].group_type, "mail_security", "security type")
for _, group in ipairs(state.contact_groups) do
  harness.assert_eq(group.source, "exchange", "source")
  harness.assert_eq(group.id, "exchange-" .. account.account_id .. "-" .. group.server_id, "id shape")
  harness.assert(group.email ~= nil, "group email")
  for _, email in ipairs(group.member_emails) do harness.assert_eq(email, string.lower(email), "lowercase member") end
end
-- The DL spans both fixture accounts, so the transitive expansion must carry
-- the second account's address alongside the owner's.
local dl_members = {}
for _, email in ipairs(by_id["grp-dl"].member_emails) do dl_members[email] = true end
harness.assert(dl_members["owner@example.test"], "DL missing owner member")
harness.assert(dl_members["member@example.test"], "DL missing cross-account member")
local req = harness.mock_requests(admin, { stable = true })
local member_of, transitive = 0, 0
for _, request in ipairs(req) do
  local command = request.command or ""
  if request.protocol == "graph" and string.find(command, "/memberOf", 1, true) then member_of = member_of + 1 end
  if request.protocol == "graph" and string.find(command, "/transitiveMembers", 1, true) then transitive = transitive + 1 end
end
harness.assert(member_of >= 1, "missing memberOf enumeration")
harness.assert_eq(transitive, 3, "one expansion per imported group")
-- Count the stable log read above, not a fresh read: re-reading here would
-- let any request that lands between the two reads into the max_delta = 0
-- budget and flake the gate.
harness.write_summary({ correct = 1, group_count = #state.contact_groups, provider_requests = #req })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown")
