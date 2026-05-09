-- description: oversized action.send JSON envelope is rejected before SMTP submission
-- fixture: jmap-small.toml
-- protocol: jmap
-- ceiling: 120s

local function smtp_log_url()
    local base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
    harness.assert(base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
    if string.sub(base, -1) == "/" then
        return base .. "test/smtp/submissions"
    end
    return base .. "/test/smtp/submissions"
end

local dir = harness.data_dir("t1_send_wire_oversize")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local log_url = smtp_log_url()
harness.http_delete(log_url)

local send_id = harness.uuid()
local oversized_body = harness.repeat_byte("z", 5 * 1024 * 1024)
local ack, ack_err = client:request("ActionSend", {
    send_id = send_id,
    from_account_id = "oversize-account",
    message = {
        draft_id = "draft-oversize",
        from = "oversize@example.test",
        to = { "recipient@example.test" },
        cc = {},
        bcc = {},
        subject = "oversize action.send envelope",
        body_html = oversized_body,
        body_text = "oversize",
    },
    attachments = {},
})
harness.assert(ack == nil, "oversize request unexpectedly acked")
harness.assert(ack_err ~= nil, "oversize request returned no error")
harness.assert_eq(ack_err.kind, "Io", "oversize error kind")
harness.assert(
    string.find(ack_err.detail, "maximum size", 1, true) ~= nil,
    "oversize error detail"
)

local submissions = harness.http_get(log_url)
harness.assert_eq(#submissions, 0, "SMTP submissions after rejection")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
