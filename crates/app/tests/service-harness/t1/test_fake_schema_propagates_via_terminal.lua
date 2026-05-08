-- description: fake schema flag turns respawn boot.ready into Terminal
-- ceiling: 90s

local function next_lifecycle(events, timeout)
    while true do
        local event = events:next(timeout)
        harness.assert(event ~= nil, "event stream closed")
        if event.type ~= "HealthChanged" then
            return event
        end
    end
end

local dir = harness.data_dir("t1_fake_schema_terminal")
local events = harness.spawn_with_events(dir)

local first = next_lifecycle(events, 5)
harness.assert_eq(first.type, "ChildSpawned", "initial ChildSpawned")
local client = first.client

local boot = next_lifecycle(events, 15)
harness.assert_eq(boot.type, "BootReady", "initial BootReady")
local baseline_schema = boot.response.schema_version
local fake_schema = baseline_schema + 1

local updated = client:set_respawn_args({
    "--test-fake-schema=" .. fake_schema,
})
harness.assert(updated, "failed to update respawn args")

local initial_pid = client:child_pid()
harness.assert(initial_pid ~= nil, "initial pid missing")
harness.kill(initial_pid, "SIGKILL")

local respawn_first = next_lifecycle(events, 20)
harness.assert_eq(respawn_first.type, "ChildSpawned", "respawn ChildSpawned")
harness.assert(harness.same_client(client, respawn_first.client), "client Arc changed")

local terminal = next_lifecycle(events, 30)
harness.assert_eq(terminal.type, "Terminal", "respawn terminal event")
harness.assert_eq(terminal.error.kind, "SchemaVersionChanged", "terminal kind")
harness.assert_eq(terminal.error.was, baseline_schema, "schema baseline")
harness.assert_eq(terminal.error.now, fake_schema, "schema override")

client:drop()
