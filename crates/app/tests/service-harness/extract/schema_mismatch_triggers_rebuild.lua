-- description: stale search index schema version triggers one post-ready rebuild
-- ceiling: 90s

-- Keep this in sync with crates/search/src/lib.rs:INDEX_SCHEMA_VERSION.
local current_search_index_schema = "2"

local function wait_for_rebuild_completed(queue, timeout)
    local deadline = harness.now_ms() + timeout * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil and notification.method == "index.rebuild_completed" then
            return notification
        end
    end
    return nil
end

local function has_rebuild_completed(notifications)
    for _, notification in ipairs(notifications) do
        if notification.method == "index.rebuild_completed" then
            return true
        end
    end
    return false
end

local function trim(text)
    return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function active_index_dir(dir)
    local pointer = dir .. "/search_index.active"
    if harness.path_exists(pointer) then
        return dir .. "/" .. trim(harness.read_text(pointer))
    end
    return dir .. "/search_index"
end

local function version_path(dir)
    return active_index_dir(dir) .. "/.version"
end

local dir = harness.data_dir("extract_schema_mismatch_rebuild")
harness.write_text(version_path(dir), "0")

local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
local queue = client:notifications()

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local completed = wait_for_rebuild_completed(queue, 30)
harness.assert(completed ~= nil, "schema mismatch rebuild did not complete")

local stored = harness.read_text(version_path(dir))
harness.assert_eq(stored, current_search_index_schema, "search index schema after rebuild")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
-- Release the first client handle before respawning against the same data dir.
client:drop()

local second, second_err = harness.spawn(dir)
harness.assert(second_err == nil, "second spawn failed")
local second_queue = second:notifications()

local second_ready, second_ready_err = second:request("BootReady")
harness.assert(second_ready_err == nil, "second boot.ready failed")
harness.assert(second_ready.ready, "second boot.ready returned ready=false")

local quiet_notifications = second_queue:drain_for(5)
harness.assert(
    not has_rebuild_completed(quiet_notifications),
    "matching schema version unexpectedly triggered another rebuild"
)
harness.assert_eq(
    harness.read_text(version_path(dir)),
    current_search_index_schema,
    "search index schema after quiet boot"
)

local second_ok, second_shutdown_err = second:shutdown()
harness.assert(second_ok, "second shutdown failed")
harness.assert(second_shutdown_err == nil, "second shutdown returned error")
