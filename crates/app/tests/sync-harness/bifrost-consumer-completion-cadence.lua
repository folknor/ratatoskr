-- description: Bifrost consumer completion-cadence API smoke
-- expected: pass
-- ceiling: 60s

local dir = harness.data_dir("bifrost_consumer_completion_cadence")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local seeded, seed_err = client:request("TestSeedAccount", {
    email = "bifrost-completion@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")

local attach, attach_err = client:request("test.bifrost_attach", {
    account_id = seeded.account_id,
    provider_kind = "jmap",
    detach_on_complete = true,
})
harness.assert(attach_err == nil, "test.bifrost_attach failed")
harness.assert(attach.subscribed, "consumer did not subscribe")
harness.assert(not attach.completed, "nonblocking attach must not complete before injection")

local injected, inject_err = client:request("test.bifrost_inject_batch", {
    account_id = seeded.account_id,
    session_id = attach.session_id,
    scope = "account",
    checkpoint = { 41 },
    messages = {
        {
            id = "bifrost-completion-m1",
            thread_id = "bifrost-completion-t1",
            subject = "Bifrost completion cadence",
            from_addr = "bifrost-completion@example.test",
            to_addrs = { "completion-peer@example.test" },
            folder_ids = { "INBOX" },
            label_ids = { "kw:completion" },
            keywords = { "completion" },
            raw_body = "completion cadence body",
        },
    },
})
harness.assert(inject_err == nil, "test.bifrost_inject_batch failed")
harness.assert(injected.acked, "completion batch should ack")

local probe, probe_err = client:request("test.bifrost_probe", {
    account_id = seeded.account_id,
    scope = "account",
    searchable_message_id = "bifrost-completion-m1",
})
harness.assert(probe_err == nil, "test.bifrost_probe failed")
harness.assert(probe.durable_cursor ~= nil, "cursor should be present after completion batch")
harness.assert_eq(probe.marker_rows, 1, "completion batch marker should be present")
harness.assert_eq(probe.is_searchable, true, "completion batch message should be persisted")

-- The completion edge for the kick above must have fired (every observed
-- scope's backfill Completed AND the 2s idle window elapsed with no batch).
-- It is surfaced through the probe's completion_edge latch.
local edge_probe = nil
for _ = 1, 10 do
    local p, p_err = client:request("test.bifrost_probe", {
        account_id = seeded.account_id,
        scope = "account",
    })
    harness.assert(p_err == nil, "completion-edge probe failed")
    if p.completion_edge == true then
        edge_probe = p
        break
    end
    harness.sleep(500)
end
harness.assert(edge_probe ~= nil, "completion edge never fired after the caught-up kick")

----------------------------------------------------------------------
-- Empty-stream edge (spec 4.1.2): a kick that injects NOTHING must still
-- reach the one-shot completion edge - with no observed scope, every
-- scope's backfill is vacuously Completed and the idle window elapses, so
-- the driver "completes immediately". This edge has no durable side
-- effect (no batch, cursor, or marker), so it is gated purely on the
-- surfaced completion_edge. A driver that hangs on an empty stream (never
-- synthesizes completion) leaves this false and fails RED.
----------------------------------------------------------------------

local empty_seeded, empty_seed_err = client:request("TestSeedAccount", {
    email = "bifrost-completion-empty@example.test",
    provider = "jmap",
})
harness.assert(empty_seed_err == nil, "TestSeedAccount (empty) failed")

local empty_attach, empty_attach_err = client:request("test.bifrost_attach", {
    account_id = empty_seeded.account_id,
    provider_kind = "jmap",
    detach_on_complete = true,
})
harness.assert(empty_attach_err == nil, "test.bifrost_attach (empty) failed")
harness.assert(empty_attach.subscribed, "empty-stream consumer did not subscribe")
harness.assert(not empty_attach.completed, "nonblocking attach must not complete synchronously")

local empty_edge = false
for _ = 1, 10 do
    local p, p_err = client:request("test.bifrost_probe", {
        account_id = empty_seeded.account_id,
        scope = "account",
    })
    harness.assert(p_err == nil, "empty-stream completion probe failed")
    if p.completion_edge == true then
        empty_edge = true
        break
    end
    harness.sleep(500)
end
harness.assert(empty_edge, "empty-stream kick never reached the completion edge")

-- The empty kick must not have manufactured any durable state.
local empty_probe, empty_probe_err = client:request("test.bifrost_probe", {
    account_id = empty_seeded.account_id,
    scope = "account",
})
harness.assert(empty_probe_err == nil, "empty-stream durable probe failed")
harness.assert(empty_probe.durable_cursor == nil, "empty-stream kick must not advance a cursor")
harness.assert_eq(empty_probe.marker_rows, 0, "empty-stream kick must write no markers")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
