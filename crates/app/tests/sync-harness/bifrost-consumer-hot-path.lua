-- description: Bifrost consumer hot-path bench API smoke
-- expected: pass
-- fixture: jmap-oauth.toml
-- protocol: jmap
-- ceiling: 60s

local dir = harness.data_dir("bifrost_consumer_hot_path")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local seeded, seed_err = client:request("TestSeedAccount", {
    email = "bifrost-hot-path@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")

local attach, attach_err = client:request("test.bifrost_attach", {
    account_id = seeded.account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err == nil, "test.bifrost_attach failed")

local message_count = 200
local messages = {}
for i = 1, message_count do
    messages[i] = {
        id = "bifrost-hot-m" .. tostring(i),
        thread_id = "bifrost-hot-t" .. tostring(i),
        subject = "Bifrost hot path " .. tostring(i),
        from_addr = "bifrost-hot-path@example.test",
        to_addrs = { "hot-peer@example.test" },
        folder_ids = { "INBOX" },
        label_ids = { "kw:hot" },
        keywords = { "hot" },
        raw_body = "hot path body " .. tostring(i),
    }
end

-- The SYNC_START/SYNC_END pair bounds the measured hot path EXACTLY to the
-- consumer draining the injected synthetic batch. brokkr derives the gate's
-- `elapsed_ms` from the span between these markers; without them it would fall
-- back to whole-run wall-clock (Service spawn + mock setup), which is not the
-- consumer hot path the gate is meant to bound (B3a-infra-spec.md § 6.1).
--
-- SYNC_END is emitted only AFTER the inject request returns acked (the consumer
-- has durably persisted + acked the checkpoint, last in the
-- hydrate -> write -> post-persist -> flush_now -> ack pipeline) and after the
-- probe confirms the drain completed (durable cursor present, every message
-- searchable, full seen count). So the marked window covers exactly the
-- consumer's batch-processing work, end-to-end through the durable ack.
local started = harness.now_ms()
harness.marker("SYNC_START")
local injected, inject_err = client:request("test.bifrost_inject_batch", {
    account_id = seeded.account_id,
    session_id = attach.session_id,
    scope = "account",
    checkpoint = { 51 },
    messages = messages,
})
harness.assert(inject_err == nil, "test.bifrost_inject_batch failed")
harness.assert(injected.acked, "hot path batch should ack")

local probe, probe_err = client:request("test.bifrost_probe", {
    account_id = seeded.account_id,
    scope = "account",
    seen_address = "hot-peer@example.test",
    searchable_message_id = "bifrost-hot-m" .. tostring(message_count),
})
harness.assert(probe_err == nil, "test.bifrost_probe failed")
harness.assert(probe.durable_cursor ~= nil, "cursor should be durable")
harness.assert_eq(probe.times_sent_to, message_count, "hot path seen count")
harness.assert_eq(probe.is_searchable, true, "last hot path message should persist")
-- The drain is confirmed durable above; close the marked hot-path window now.
harness.marker("SYNC_END")

local elapsed_ms = math.max(1, harness.now_ms() - started)
harness.write_summary({
    messages_per_second = (message_count * 1000) / elapsed_ms,
    marker_rows = probe.marker_rows,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
