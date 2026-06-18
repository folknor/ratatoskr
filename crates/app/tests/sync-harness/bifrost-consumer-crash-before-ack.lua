-- description: Bifrost consumer crash-before-ack survives a real Service reboot + re-inject
-- expected: pass
-- ceiling: 90s

-- This gate proves the durability contract of spec 4.1.3 (ack-last +
-- the single-txn replay-safety marker) across a TRUE crash-replay: the
-- consumer persists rows + post-persist (seen-ingest + marker) + search
-- flush, the CrashBeforeAck hook exits the drive task BEFORE the ack, the
-- whole Service is then shut down and RE-SPAWNED against the same data
-- dir, and the identical batch is re-injected (modelling the engine's
-- at-least-once re-delivery of a checkpoint whose ack never landed). The
-- assertion is that across the reboot the durable cursor never advanced
-- for the un-acked batch and the seen-counter reflects a SINGLE ingest,
-- not a double - i.e. the marker, persisted in the main DB by the same
-- txn as the increment, suppressed the post-reboot re-increment.

local dir = harness.data_dir("bifrost_consumer_crash_before_ack")

----------------------------------------------------------------------
-- Incarnation A: persist + post-persist + flush, crash BEFORE the ack.
----------------------------------------------------------------------

local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "spawn A failed")

local ready_a, ready_err_a = client_a:request("BootReady")
harness.assert(ready_err_a == nil, "boot.ready A failed")
harness.assert(ready_a.ready, "boot.ready A returned ready=false")

local seeded, seed_err = client_a:request("TestSeedAccount", {
    email = "bifrost-crash-before-ack@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")
local account_id = seeded.account_id

local armed, arm_err = client_a:request("test.bifrost_arm_hook", {
    account_id = account_id,
    hook = { kind = "crash_before_ack" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "hook was not armed")

local attach_a, attach_err_a = client_a:request("test.bifrost_attach", {
    account_id = account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err_a == nil, "test.bifrost_attach A failed")

-- The synthetic batch is identical across both incarnations so the
-- re-inject on incarnation B is a faithful re-delivery of the same
-- (scope, checkpoint).
local batch_messages = {
    {
        id = "bifrost-crash-m1",
        thread_id = "bifrost-crash-t1",
        subject = "Bifrost crash before ack",
        from_addr = "bifrost-crash-before-ack@example.test",
        to_addrs = { "crash-peer@example.test" },
        folder_ids = { "INBOX" },
        label_ids = { "kw:crash" },
        keywords = { "crash" },
        raw_body = "crash before ack body",
    },
}

local injected_a, inject_err_a = client_a:request("test.bifrost_inject_batch", {
    account_id = account_id,
    session_id = attach_a.session_id,
    scope = "account",
    checkpoint = { 21 },
    messages = batch_messages,
})
harness.assert(inject_err_a == nil, "test.bifrost_inject_batch A failed")
harness.assert(injected_a.acked, "inject request should represent the intended checkpoint")

-- Before the reboot, confirm incarnation A reached the pre-ack state:
-- the rows + marker committed, the seen-counter incremented once, and the
-- ack was WITHHELD (no durable cursor) by the crash hook.
local probe_a, probe_err_a = client_a:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "account",
    seen_address = "crash-peer@example.test",
    searchable_message_id = "bifrost-crash-m1",
})
harness.assert(probe_err_a == nil, "test.bifrost_probe A failed")
harness.assert(probe_a.durable_cursor == nil, "crash-before-ack must withhold cursor (incarnation A)")
harness.assert_eq(probe_a.marker_rows, 1, "marker must commit before the withheld ack")
harness.assert_eq(probe_a.times_sent_to, 1, "seen ingest should commit once in incarnation A")
harness.assert_eq(probe_a.is_searchable, true, "message should persist before the withheld ack")

local ok_a, shutdown_err_a = client_a:shutdown()
harness.assert(ok_a, "shutdown A failed")
harness.assert(shutdown_err_a == nil, "shutdown A returned error")
client_a:drop()

----------------------------------------------------------------------
-- Incarnation B: reboot the SAME data dir, re-inject the SAME batch.
-- The engine's at-least-once redelivery is modelled by a fresh attach +
-- re-inject of the un-acked (scope, checkpoint). The marker - durable in
-- the main DB across the reboot - must suppress the seen re-increment.
----------------------------------------------------------------------

local client_b, err_b = harness.spawn(dir)
harness.assert(err_b == nil, "spawn B failed")

local ready_b, ready_err_b = client_b:request("BootReady")
harness.assert(ready_err_b == nil, "boot.ready B failed")
harness.assert(ready_b.ready, "boot.ready B returned ready=false")

-- Sanity: the pre-reboot durable state survived the crash + reboot. The
-- cursor is still unadvanced (the ack never landed in incarnation A), the
-- marker row persisted, and the seen-counter is still a single ingest.
local probe_reboot, probe_reboot_err = client_b:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "account",
    seen_address = "crash-peer@example.test",
    searchable_message_id = "bifrost-crash-m1",
})
harness.assert(probe_reboot_err == nil, "test.bifrost_probe (post-reboot) failed")
harness.assert(probe_reboot.durable_cursor == nil, "cursor must still be unadvanced after reboot")
harness.assert_eq(probe_reboot.marker_rows, 1, "marker must survive the reboot")
harness.assert_eq(probe_reboot.times_sent_to, 1, "seen counter must survive the reboot as a single ingest")
harness.assert_eq(probe_reboot.is_searchable, true, "message must remain durable after reboot")

-- No crash hook armed this time: the re-inject runs the full pipeline to
-- the ack. The marker (present from incarnation A) suppresses the seen
-- re-increment, so the counter STAYS at one even though the batch was
-- delivered twice across a real process boundary.
local attach_b, attach_err_b = client_b:request("test.bifrost_attach", {
    account_id = account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err_b == nil, "test.bifrost_attach B failed")

local injected_b, inject_err_b = client_b:request("test.bifrost_inject_batch", {
    account_id = account_id,
    session_id = attach_b.session_id,
    scope = "account",
    checkpoint = { 21 },
    messages = batch_messages,
})
harness.assert(inject_err_b == nil, "test.bifrost_inject_batch B failed")
harness.assert(injected_b.acked, "re-inject should ack (no crash hook on incarnation B)")

local probe_b, probe_err_b = client_b:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "account",
    seen_address = "crash-peer@example.test",
    searchable_message_id = "bifrost-crash-m1",
})
harness.assert(probe_err_b == nil, "test.bifrost_probe B failed")
harness.assert(probe_b.durable_cursor ~= nil, "cursor must advance on the un-crashed re-delivery")
harness.assert_eq(probe_b.durable_cursor.kind, "change", "re-delivery cursor kind")
harness.assert_eq(probe_b.durable_cursor.scope_key, "account", "re-delivery cursor scope")
harness.assert_eq(probe_b.times_sent_to, 1,
    "marker must suppress the post-reboot re-increment - a single ingest across two deliveries")
harness.assert(probe_b.marker_rows >= 1, "marker must remain after the acked re-delivery")
harness.assert_eq(probe_b.is_searchable, true, "message remains searchable after re-delivery")

local ok_b, shutdown_err_b = client_b:shutdown()
harness.assert(ok_b, "shutdown B failed")
harness.assert(shutdown_err_b == nil, "shutdown B returned error")
