-- description: JMAP foreign account sync persists namespaced mail with a body
-- expected: pass
-- fixture: shared-jmap-small.toml
-- protocol: jmap
-- ceiling: 120s
-- @covers: bifrost.b12.jmap_shared_account

local dir = harness.data_dir("jmap_shared_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "primary@example.test",
    display_name = "Primary",
    account_name = "Primary",
    provider = "jmap",
})
harness.assert(account_err == nil, "seed failed")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and completed.result == "completed", completed.error or "sync failed")
local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(state_err == nil, "state read failed")
local shared = nil
for _, message in ipairs(state.messages) do
    if message.subject == "Shared copy" then shared = message end
end
harness.assert(shared ~= nil, "foreign message missing")
harness.assert(shared.id ~= "team-message", "foreign message storage id was not namespaced")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
