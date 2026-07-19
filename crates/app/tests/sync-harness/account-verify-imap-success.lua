-- description: account.verify opens and closes an IMAP mailbox without persisting an account
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: imap
-- ceiling: 120s

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local client, err = harness.spawn(harness.data_dir("account_verify_imap_success"))
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
    imap_password = "password",
    accept_invalid_certs = false,
})
harness.assert(verify_err == nil, "account.verify transport failed")
harness.assert(ack.ok, ack.message or "account.verify failed")

local state, state_err = client:request("TestQueryDbState", {})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.account_count, 0, "verify persisted an account row")

local requests = harness.mock_requests(admin_endpoint, { stable = true })
harness.assert(harness.request_count(requests, "imap", "LOGIN") >= 1, "expected LOGIN")
harness.assert(harness.request_count(requests, "imap", "LIST") >= 1, "expected LIST")
harness.assert(harness.request_count(requests, "imap", "LOGOUT") >= 1, "expected LOGOUT from close")

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
