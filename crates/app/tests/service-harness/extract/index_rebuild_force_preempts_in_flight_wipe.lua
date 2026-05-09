-- description: forced index rebuild preempts an in-flight wipe rebuild
-- ceiling: 90s

local function wait_for_rebuild_completed(queue, rebuild_id, forbidden_id, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "index.rebuild_completed"
        then
            local completed_id = notification.rebuild_id
            harness.assert(
                completed_id ~= forbidden_id,
                "preempted rebuild completed"
            )
            if completed_id == rebuild_id then
                return notification
            end
        end
    end
    return nil
end

local dir = harness.data_dir("extract_index_rebuild_force_preempts_wipe")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local _, delay_err = client:request("TestDelayNextWrite", {
    kind = "search.clear",
    millis = 1000,
})
harness.assert(delay_err == nil, "search.clear delay hook failed")

local first, first_err = client:request("IndexRebuild", {
    policy = "wipe",
    force = false,
})
harness.assert(first_err == nil, "first index.rebuild failed")
harness.assert(first.rebuild_id ~= nil, "first rebuild_id missing")

local duplicate, duplicate_err = client:request("IndexRebuild", {
    policy = "wipe",
    force = false,
})
harness.assert(duplicate == nil, "duplicate index.rebuild unexpectedly succeeded")
harness.assert(duplicate_err ~= nil, "duplicate index.rebuild missing error")
harness.assert_eq(duplicate_err.kind, "Service", "duplicate error kind")
harness.assert(
    string.find(duplicate_err.detail, "already in flight", 1, true) ~= nil,
    "duplicate error detail"
)

local second, second_err = client:request("IndexRebuild", {
    policy = "wipe",
    force = true,
})
harness.assert(second_err == nil, "forced index.rebuild failed")
harness.assert(second.rebuild_id ~= nil, "forced rebuild_id missing")
harness.assert(
    second.rebuild_id ~= first.rebuild_id,
    "forced rebuild reused first rebuild_id"
)

local completed = wait_for_rebuild_completed(
    queue,
    second.rebuild_id,
    first.rebuild_id,
    30
)
harness.assert(completed ~= nil, "forced rebuild did not complete")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
