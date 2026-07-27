-- description: IMAP shared folders persist namespaced mail without colliding with personal mail
-- expected: pass
-- fixture: shared-imap-small.toml
-- protocol: imap
-- ceiling: 120s
-- @covers: bifrost.b12.imap_shared_folder

local dir = harness.data_dir("imap_shared_folder")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer",
    account_name = "Viewer", provider = "imap",
})
harness.assert(account_err == nil, "seed failed")
local completed, sync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(sync_err == nil and completed.result == "completed", completed.error or "sync failed")
local state, state_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(state_err == nil, "state read failed")
local personal, shared = nil, nil
for _, message in ipairs(state.messages) do
    if message.subject == "Personal copy" then personal = message end
    if message.subject == "Shared copy" then shared = message end
end
harness.assert(personal ~= nil and shared ~= nil, "personal and shared messages must both persist")
harness.assert(personal.thread_id ~= shared.thread_id, "shared Message-ID must not collide with personal thread")
harness.assert(shared.id ~= "alice-copy", "shared storage id must be namespace qualified")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
