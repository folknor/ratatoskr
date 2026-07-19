-- description: account.verify maps an IMAP authentication rejection without persisting an account
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: imap
-- ceiling: 120s

local client, err = harness.spawn(harness.data_dir("account_verify_imap_bad_password"))
harness.assert(err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot.ready failed")

local ack, verify_err = client:request("account.verify", {
    provider = "imap",
    email = "primary@example.com",
    imap_host = "imap.example.test",
    imap_port = 993,
    imap_security = "tls",
    username = "primary@example.com",
    imap_password = "saehrimnir-reject-auth",
    accept_invalid_certs = false,
})
harness.assert(verify_err == nil, "account.verify transport failed")
harness.assert(not ack.ok, "bad password unexpectedly verified")
harness.assert(ack.message ~= nil and #ack.message > 0, "missing verify error message")

local state, state_err = client:request("TestQueryDbState", {})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.account_count, 0, "verify persisted an account row")

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
