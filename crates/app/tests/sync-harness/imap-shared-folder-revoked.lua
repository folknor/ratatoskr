-- description: IMAP ACL revocation quarantines only the shared scope
-- expected: pass
-- fixture: imap-acl-lifecycle.toml
-- protocol: imap
-- ceiling: 120s
-- @covers: bifrost.b12.imap_shared_folder_revoked

local function sync(client, account_id, label)
    local result, err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

local function step(endpoint, id)
    local result = harness.http_json({
        method = "POST", url = harness.join_url(endpoint, "test/fixture/step"), body = { expect = id },
    })
    harness.assert(result.ok, id .. " fixture step failed")
    harness.assert_eq(result.step, id, id .. " fixture step")
end

local function find_message(state, subject)
    for _, message in ipairs(state.messages) do
        if message.subject == subject then return message end
    end
    return nil
end

local endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(endpoint ~= nil, "saehrimnir admin endpoint missing")
local dir = harness.data_dir("imap_shared_folder_revoked")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")
local account, account_err = client:request("TestSeedAccount", {
    email = "viewer@example.test", display_name = "Viewer", account_name = "Viewer", provider = "imap",
})
harness.assert(account_err == nil, "seed failed")

step(endpoint, "grant")
sync(client, account.account_id, "shared initial")
local before, before_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(before_err == nil, "state before revoke failed")
local shared = find_message(before, "ACL shared message")
harness.assert(shared ~= nil, "granted shared mail missing")

step(endpoint, "revoke")
sync(client, account.account_id, "revocation")
local after_revoke, revoke_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(revoke_err == nil, "state after revoke failed")
harness.assert(find_message(after_revoke, "ACL shared message") ~= nil, "revocation removed durable shared mail")

step(endpoint, "personal-delta")
sync(client, account.account_id, "personal delta after revocation")
local final, final_err = client:request("TestQueryDbState", { account_id = account.account_id, message_limit = 20 })
harness.assert(final_err == nil, "final state failed")
harness.assert(find_message(final, "Personal after revoke") ~= nil, "shared revocation stopped personal sync")
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
