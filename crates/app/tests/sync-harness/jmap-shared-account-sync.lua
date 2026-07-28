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

-- B15d: `reconcile_namespace_registry` registers the foreign account and, in
-- the same transaction, projects `Container.owner_email` into
-- `shared_mailboxes.email_address` (the pop-out compose sender identity).
local function shared_row(rows)
    for _, row in ipairs(rows) do
        if row.mailbox_id == "account-team" then return row end
    end
    return nil
end
local registry = shared_row(state.shared_mailboxes)
harness.assert(registry ~= nil, "foreign account was not registered in shared_mailboxes")

-- The owner email resolves through bifrost's two-level RFC 9670 gate: the
-- SESSION must advertise `urn:ietf:params:jmap:principals` and the ACCOUNT the
-- `:owner` extension with a `principalId`, which saehrimnir serves as of
-- df9a300. That path fails SOFT - a missing capability or a misshapen
-- `Principal/get` yields `owner_email = nil` rather than an error - so this
-- asserts the resolved value rather than mere presence, or the gate would go
-- quietly green again the moment the mock stopped advertising.
harness.assert(
    registry.email_address == "team@example.test",
    "owner email did not resolve through the principals gate (got "
        .. tostring(registry.email_address) .. ")"
)

-- Write-once: `reconcile_namespace_registry` fills `email_address` only while
-- it IS NULL, so a second sync must not rewrite it. Re-sync and re-read.
local recompleted, resync_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(
    resync_err == nil and recompleted.result == "completed",
    recompleted.error or "re-sync failed"
)
local restate, restate_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    message_limit = 20,
})
harness.assert(restate_err == nil, "state re-read failed")
local reregistry = shared_row(restate.shared_mailboxes)
harness.assert(reregistry ~= nil, "shared_mailboxes row vanished across re-sync")
harness.assert(
    reregistry.email_address == "team@example.test",
    "owner email changed across re-sync - the write-once guard is not holding"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
