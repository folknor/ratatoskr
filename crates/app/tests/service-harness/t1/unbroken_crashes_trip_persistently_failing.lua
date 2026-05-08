-- description: unbroken respawn crashes trip PersistentlyFailing
-- ceiling: 120s

local function assert_not_persistently_failing(event)
    if event.type == "HealthChanged" then
        local marker = string.find(event.health, "PersistentlyFailing", 1, true)
        harness.assert(marker == nil, "unexpected PersistentlyFailing health")
    end
end

local function wait_for_initial_ready(events)
    local first = events:next(5)
    harness.assert(first ~= nil, "event stream closed")
    harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
    local client = first.client

    while true do
        local event = events:next(15)
        harness.assert(event ~= nil, "event stream closed")
        assert_not_persistently_failing(event)
        if event.type == "BootReady" then
            return client
        end
        harness.assert(event.type ~= "Terminal", "initial boot reached Terminal")
    end
end

local function wait_for_recovery(events, client)
    local saw_child = false
    while true do
        local event = events:next(30)
        harness.assert(event ~= nil, "event stream closed")
        assert_not_persistently_failing(event)
        if event.type == "ChildSpawned" then
            harness.assert(harness.same_client(client, event.client), "client Arc changed")
            saw_child = true
        elseif event.type == "BootReady" then
            harness.assert(saw_child, "BootReady arrived before ChildSpawned")
            return
        else
            harness.assert(event.type ~= "Terminal", "recovery reached Terminal")
        end
    end
end

local function exercise_recovered_crashes()
    local dir = harness.data_dir("t1_recovered_crashes_do_not_trip")
    local events = harness.spawn_with_events(dir)
    local client = wait_for_initial_ready(events)

    for _ = 1, 3 do
        local pid = client:child_pid()
        harness.assert(pid ~= nil, "pid missing")
        harness.kill(pid, "SIGKILL")
        wait_for_recovery(events, client)
    end

    local quiet = harness.expect_quiet(events, 1)
    harness.assert(quiet, "terminal event after recovered crashes")

    local ok, shutdown_err = client:shutdown()
    harness.assert(ok, "shutdown failed")
    harness.assert(shutdown_err == nil, "shutdown returned error")
end

local function exercise_unbroken_crashes()
    local dir = harness.data_dir("t1_unbroken_crashes_trip")
    local events = harness.spawn_with_events(dir)
    local client = wait_for_initial_ready(events)

    local updated = client:set_respawn_args({
        "--test-boot-delay-ms=5000",
    })
    harness.assert(updated, "failed to update respawn args")

    local pid = client:child_pid()
    harness.assert(pid ~= nil, "pid missing")
    harness.kill(pid, "SIGKILL")

    local saw_persistent = false
    local killed_respawns = 0
    local terminal = nil
    local deadline = harness.now_ms() + 30000
    while harness.now_ms() < deadline do
        local event = events:next(10)
        harness.assert(event ~= nil, "event stream closed")
        if event.type == "HealthChanged" then
            local marker = string.find(event.health, "PersistentlyFailing", 1, true)
            if marker ~= nil then
                saw_persistent = true
            end
        elseif event.type == "ChildSpawned" then
            harness.assert(harness.same_client(client, event.client), "client Arc changed")
            local respawn_pid = event.client:child_pid()
            harness.assert(respawn_pid ~= nil, "respawn pid missing")
            harness.kill(respawn_pid, "SIGKILL")
            killed_respawns = killed_respawns + 1
        elseif event.type == "BootReady" then
            harness.assert(false, "unbroken crashloop recovered unexpectedly")
        elseif event.type == "Terminal" then
            terminal = event
            break
        end
    end

    harness.assert(terminal ~= nil, "missing Terminal after unbroken crashes")
    harness.assert(saw_persistent, "PersistentlyFailing health was not emitted")
    harness.assert(killed_respawns >= 2, "did not kill two respawn attempts")
    harness.assert_eq(terminal.error.kind, "BootFailure", "terminal kind")
    harness.assert_eq(
        terminal.error.classification,
        "UnexpectedExit",
        "terminal classification"
    )

    client:drop()
end

exercise_recovered_crashes()
exercise_unbroken_crashes()
