-- description: Bifrost consumer lag-recovery API smoke
-- expected: pass
-- ceiling: 60s

local dir = harness.data_dir("bifrost_consumer_lag_recovery")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local seeded, seed_err = client:request("TestSeedAccount", {
    email = "bifrost-lag@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")

local armed, arm_err = client:request("test.bifrost_arm_hook", {
    account_id = seeded.account_id,
    hook = { kind = "stall_consumer", after_ms = 800 },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "hook was not armed")

local attach, attach_err = client:request("test.bifrost_attach", {
    account_id = seeded.account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err == nil, "test.bifrost_attach failed")

local first, first_err = client:request("test.bifrost_inject_batch", {
    account_id = seeded.account_id,
    session_id = attach.session_id,
    scope = "account",
    checkpoint = { 61 },
    messages = {
        {
            id = "bifrost-lag-m1",
            thread_id = "bifrost-lag-t1",
            subject = "Bifrost lag before gap",
            from_addr = "bifrost-lag@example.test",
            to_addrs = { "lag-peer@example.test" },
            folder_ids = { "INBOX" },
            label_ids = { "kw:lag" },
            keywords = { "lag" },
            raw_body = "lag before gap",
        },
    },
})
harness.assert(first_err == nil, "first inject failed")
harness.assert(first.acked, "first batch should ack")

local requests = {}
for i = 2, 24 do
    requests[i] = client:request_async("test.bifrost_inject_batch", {
        account_id = seeded.account_id,
        session_id = attach.session_id,
        scope = "account",
        checkpoint = { 60 + i },
        messages = {
            {
                id = "bifrost-lag-m" .. tostring(i),
                thread_id = "bifrost-lag-t" .. tostring(i),
                subject = "Bifrost lag gap " .. tostring(i),
                from_addr = "bifrost-lag@example.test",
                to_addrs = { "lag-peer@example.test" },
                folder_ids = { "INBOX" },
                label_ids = { "kw:lag" },
                keywords = { "lag" },
                raw_body = "lag gap " .. tostring(i),
            },
        },
    })
end
for i = 2, 24 do
    local _, req_err = requests[i]:await(5)
    if req_err ~= nil then
        -- Once the bounded broadcast lags, the consumer stops and later
        -- inject calls can time out waiting for intentionally dropped rows.
        local _ignored = req_err
    end
end

local probe, probe_err = client:request("test.bifrost_probe", {
    account_id = seeded.account_id,
    scope = "account",
    seen_address = "lag-peer@example.test",
    searchable_message_id = "bifrost-lag-m1",
})
harness.assert(probe_err == nil, "test.bifrost_probe failed")
harness.assert(probe.durable_cursor ~= nil, "lag gate should keep last safe cursor")
harness.assert_eq(probe.durable_cursor.kind, "change", "cursor kind")
harness.assert_eq(probe.is_searchable, true, "pre-gap message should persist")
harness.assert(probe.times_sent_to >= 1, "seen counter should include pre-gap message")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
