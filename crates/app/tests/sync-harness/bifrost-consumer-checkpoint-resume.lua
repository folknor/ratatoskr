-- description: Bifrost consumer checkpoint-resume API smoke
-- expected: pass
-- ceiling: 60s

local dir = harness.data_dir("bifrost_consumer_checkpoint_resume")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local seeded, seed_err = client:request("TestSeedAccount", {
    email = "bifrost-checkpoint@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")

local attach, attach_err = client:request("test.bifrost_attach", {
    account_id = seeded.account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err == nil, "test.bifrost_attach failed")
harness.assert(attach.subscribed, "consumer did not subscribe")

local injected, inject_err = client:request("test.bifrost_inject_batch", {
    account_id = seeded.account_id,
    session_id = attach.session_id,
    scope = "account",
    checkpoint = { 11 },
    messages = {
        {
            id = "bifrost-checkpoint-m1",
            thread_id = "bifrost-checkpoint-t1",
            subject = "Bifrost checkpoint resume",
            from_addr = "bifrost-checkpoint@example.test",
            to_addrs = { "resume-peer@example.test" },
            folder_ids = { "INBOX" },
            label_ids = { "kw:checkpoint" },
            keywords = { "checkpoint" },
            raw_body = "checkpoint resume searchable body",
        },
    },
})
harness.assert(inject_err == nil, "test.bifrost_inject_batch failed")
harness.assert(injected.acked, "batch should ack")

local probe, probe_err = client:request("test.bifrost_probe", {
    account_id = seeded.account_id,
    scope = "account",
    seen_address = "resume-peer@example.test",
    searchable_message_id = "bifrost-checkpoint-m1",
})
harness.assert(probe_err == nil, "test.bifrost_probe failed")
harness.assert(probe.durable_cursor ~= nil, "durable cursor missing after ack")
harness.assert_eq(probe.durable_cursor.kind, "change", "cursor kind")
harness.assert_eq(probe.durable_cursor.scope_key, "account", "cursor scope")
harness.assert_eq(probe.times_sent_to, 1, "seen counter should be exactly once")
harness.assert_eq(probe.is_searchable, true, "message should be queryable after inject")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
