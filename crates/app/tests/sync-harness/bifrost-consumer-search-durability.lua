-- description: Bifrost consumer search durability survives a real Service reboot after ack
-- expected: pass
-- ceiling: 90s

-- This gate proves spec 4.1.3's search-flush-before-ack boundary across a
-- TRUE reboot: the consumer queues the search docs, COMMITS Tantivy via
-- flush_now, THEN acks; the CrashAfterAckNoSentinel hook exits the drive
-- task with no clean-shutdown sentinel; the whole Service is shut down and
-- RE-SPAWNED against the same data dir. The post-reboot assertion is that
-- every injected message is still searchable - the committed index
-- survived the crash even though the cursor had already advanced. A
-- regression that acks on queued-but-uncommitted docs (skips flush_now)
-- would lose them at the crash and fail the post-reboot search.

local dir = harness.data_dir("bifrost_consumer_search_durability")

----------------------------------------------------------------------
-- Incarnation A: inject, flush+ack, then crash after the ack.
----------------------------------------------------------------------

local client_a, err_a = harness.spawn(dir)
harness.assert(err_a == nil, "spawn A failed")

local ready_a, ready_err_a = client_a:request("BootReady")
harness.assert(ready_err_a == nil, "boot.ready A failed")
harness.assert(ready_a.ready, "boot.ready A returned ready=false")

local seeded, seed_err = client_a:request("TestSeedAccount", {
    email = "bifrost-search@example.test",
    provider = "jmap",
})
harness.assert(seed_err == nil, "TestSeedAccount failed")
local account_id = seeded.account_id

local armed, arm_err = client_a:request("test.bifrost_arm_hook", {
    account_id = account_id,
    hook = { kind = "crash_after_ack_no_sentinel" },
})
harness.assert(arm_err == nil, "test.bifrost_arm_hook failed")
harness.assert(armed.armed, "hook was not armed")

local attach_a, attach_err_a = client_a:request("test.bifrost_attach", {
    account_id = account_id,
    provider_kind = "jmap",
    detach_on_complete = false,
})
harness.assert(attach_err_a == nil, "test.bifrost_attach A failed")

local injected, inject_err = client_a:request("test.bifrost_inject_batch", {
    account_id = account_id,
    session_id = attach_a.session_id,
    scope = "account",
    checkpoint = { 31 },
    messages = {
        {
            id = "bifrost-search-m1",
            thread_id = "bifrost-search-t1",
            subject = "Bifrost search durability",
            from_addr = "bifrost-search@example.test",
            to_addrs = { "search-peer@example.test" },
            folder_ids = { "INBOX" },
            label_ids = { "kw:search" },
            keywords = { "search" },
            raw_body = "needle-bifrost-search-durability",
        },
    },
})
harness.assert(inject_err == nil, "test.bifrost_inject_batch failed")
harness.assert(injected.acked, "batch should ack before crash-after-ack hook exits task")

-- Shut down the Service entirely. The CrashAfterAckNoSentinel hook
-- already exited the drive task with no clean-shutdown sentinel after the
-- ack; the search index commit (flush_now) happened BEFORE that ack, so
-- the docs must be on disk independent of any further clean shutdown.
local ok_a, shutdown_err_a = client_a:shutdown()
harness.assert(ok_a, "shutdown A failed")
harness.assert(shutdown_err_a == nil, "shutdown A returned error")
client_a:drop()

----------------------------------------------------------------------
-- Incarnation B: reboot the SAME data dir; the committed search docs and
-- the durable cursor must both be intact.
----------------------------------------------------------------------

local client_b, err_b = harness.spawn(dir)
harness.assert(err_b == nil, "spawn B failed")

local ready_b, ready_err_b = client_b:request("BootReady")
harness.assert(ready_err_b == nil, "boot.ready B failed")
harness.assert(ready_b.ready, "boot.ready B returned ready=false")

local probe, probe_err = client_b:request("test.bifrost_probe", {
    account_id = account_id,
    scope = "account",
    searchable_message_id = "bifrost-search-m1",
})
harness.assert(probe_err == nil, "test.bifrost_probe failed")
harness.assert(probe.durable_cursor ~= nil, "cursor should be durable across reboot after ack")
harness.assert_eq(probe.is_searchable, true, "message should be persisted across reboot")

-- The load-bearing assertion: the Tantivy index, committed by flush_now
-- BEFORE the ack, returns the injected doc after a real reboot. A
-- regression that acks on queued-but-uncommitted docs loses them here.
local search, search_err = client_b:request("test.search_index", {
    account_id = account_id,
    query = "needle-bifrost-search-durability",
})
harness.assert(search_err == nil, "test.search_index failed")
harness.assert(search.total >= 1, "search index did not return injected document after reboot")

local ok_b, shutdown_err_b = client_b:shutdown()
harness.assert(ok_b, "shutdown B failed")
harness.assert(shutdown_err_b == nil, "shutdown B returned error")
