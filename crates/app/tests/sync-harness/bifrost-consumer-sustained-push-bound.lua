-- description: Bifrost resident consumer bounds sustained-push accumulators
-- expected: pass
-- ceiling: 120s

local dir = harness.data_dir("bifrost_consumer_sustained_push_bound")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local graph, graph_err = client:request("TestSeedAccount", {
    email = "bifrost-sustained-graph@example.test",
    provider = "graph",
})
harness.assert(graph_err == nil, "TestSeedAccount graph failed")

local graph_attach, graph_attach_err = client:request("test.bifrost_attach", {
    account_id = graph.account_id,
    provider_kind = "graph",
    resident = true,
    detach_on_complete = false,
})
harness.assert(graph_attach_err == nil, "test.bifrost_attach graph failed")

local removed = {}
for i = 1, 4097 do
    removed[i] = {
        id = "bifrost-sustained-graph-" .. tostring(i),
        change_kind = "scope_removed",
    }
end

local graph_inject, graph_inject_err = client:request("test.bifrost_inject_batch", {
    account_id = graph.account_id,
    session_id = graph_attach.session_id,
    scope = "account",
    checkpoint = { 1 },
    messages = removed,
})
harness.assert(graph_inject_err == nil, "graph sustained removed batch failed")
harness.assert(graph_inject.acked, "graph sustained removed batch should ack")

local graph_probe, graph_probe_err = client:request("test.bifrost_probe", {
    account_id = graph.account_id,
    scope = "account",
})
harness.assert(graph_probe_err == nil, "graph probe failed")
harness.assert(
    graph_probe.resident_forced_flushes >= 1,
    "graph pending-deletion cap did not force a resident flush"
)
harness.assert(
    graph_probe.resident_max_pending_deletions >= 4096,
    "graph pending-deletion accumulator did not reach the cap"
)
harness.assert(graph_probe.durable_cursor ~= nil, "graph checkpoint did not advance")

local imap, imap_err = client:request("TestSeedAccount", {
    email = "bifrost-sustained-imap@example.test",
    provider = "imap",
})
harness.assert(imap_err == nil, "TestSeedAccount imap failed")

local imap_attach, imap_attach_err = client:request("test.bifrost_attach", {
    account_id = imap.account_id,
    provider_kind = "imap",
    resident = true,
    detach_on_complete = false,
})
harness.assert(imap_attach_err == nil, "test.bifrost_attach imap failed")

for i = 1, 129 do
    local inject, inject_err = client:request("test.bifrost_inject_batch", {
        account_id = imap.account_id,
        session_id = imap_attach.session_id,
        scope = "folder:INBOX",
        checkpoint = { i },
        await_ack = false,
        messages = {
            {
                id = "bifrost-sustained-imap-" .. tostring(i),
                thread_id = "bifrost-sustained-imap-t" .. tostring(i),
                subject = "Bifrost sustained IMAP " .. tostring(i),
                from_addr = "bifrost-sustained@example.test",
                to_addrs = { "imap-peer@example.test" },
                folder_ids = { "INBOX" },
                raw_body = "sustained imap " .. tostring(i),
            },
        },
    })
    harness.assert(inject_err == nil, "imap sustained inject failed")
    harness.assert(inject.acked, "imap sustained batch should be ackable")
end

local imap_probe = nil
for _ = 1, 20 do
    local probe, probe_err = client:request("test.bifrost_probe", {
        account_id = imap.account_id,
        scope = "folder:INBOX",
        searchable_message_id = "bifrost-sustained-imap-129",
    })
    harness.assert(probe_err == nil, "imap probe failed")
    imap_probe = probe
    if probe.resident_forced_flushes >= 1 and probe.durable_cursor ~= nil then
        break
    end
    harness.sleep(0.25)
end

harness.assert(imap_probe ~= nil, "imap probe missing")
harness.assert(
    imap_probe.resident_forced_flushes >= 1,
    "imap deferred-ack cap did not force a resident flush"
)
harness.assert(
    imap_probe.resident_max_deferred_acks >= 128,
    "imap deferred-ack accumulator did not reach the cap"
)
harness.assert(imap_probe.durable_cursor ~= nil, "imap checkpoint did not advance")
harness.assert_eq(imap_probe.is_searchable, true, "imap last message should persist")

harness.write_summary({
    correct = 1,
    graph_forced_flushes = graph_probe.resident_forced_flushes,
    graph_max_pending_deletions = graph_probe.resident_max_pending_deletions,
    imap_forced_flushes = imap_probe.resident_forced_flushes,
    imap_max_deferred_acks = imap_probe.resident_max_deferred_acks,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
