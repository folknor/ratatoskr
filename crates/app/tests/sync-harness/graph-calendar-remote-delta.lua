-- description: Graph calendar remote mutations are imported by delta sync
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: graph
-- ceiling: 120s

local function calendar_by_remote_id(calendars, remote_id)
    for _, calendar in ipairs(calendars) do
        if calendar.remote_id == remote_id then
            return calendar
        end
    end
    return nil
end

local function event_by_remote_id(events, remote_id)
    for _, event in ipairs(events) do
        if event.remote_event_id == remote_id then
            return event
        end
    end
    return nil
end

local function graph_url(base, suffix)
    if string.sub(base, -5) == "/v1.0" then
        return harness.join_url(base, suffix)
    end
    return harness.join_url(harness.join_url(base, "v1.0"), suffix)
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local graph_endpoint = harness.env("RATATOSKR_TEST_GRAPH_ENDPOINT")
harness.assert(graph_endpoint ~= nil, "RATATOSKR_TEST_GRAPH_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_graph_calendar_remote_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-graph-calendar-delta@example.test",
    display_name = "Sync Graph Calendar Delta",
    account_name = "Sync Graph Calendar Delta",
    provider = "graph",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial_sync, initial_sync_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.assert(initial_sync_err == nil, "initial start_calendar_sync failed")
harness.assert_eq(
    initial_sync.result,
    "completed",
    initial_sync.error or "initial calendar sync result"
)

local initial, initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(initial_err == nil, "initial TestQueryDbState failed")
harness.assert_eq(initial.calendar_count, 2, "initial calendar count")
harness.assert_eq(initial.calendar_event_count, 2, "initial event count")
local work = calendar_by_remote_id(initial.calendars, "cal-work")
harness.assert(work ~= nil, "missing Work calendar")
local standup = event_by_remote_id(initial.calendar_events, "ev-001")
harness.assert(standup ~= nil, "missing Standup before mutation")
local review = event_by_remote_id(initial.calendar_events, "ev-002")
harness.assert(review ~= nil, "missing Quarterly review before mutation")

local created = harness.http_json({
    method = "POST",
    url = graph_url(graph_endpoint, "me/calendars/cal-work/events"),
    body = {
        subject = "Remote created",
        body = {
            contentType = "text",
            content = "Created directly through the mock Graph endpoint",
        },
        location = {
            displayName = "Remote Room",
        },
        start = {
            dateTime = "2026-02-10T10:00:00Z",
            timeZone = "UTC",
        },
        ["end"] = {
            dateTime = "2026-02-10T10:30:00Z",
            timeZone = "UTC",
        },
        isAllDay = false,
    },
})
harness.assert(created ~= nil, "Graph create returned no body")
harness.assert(created.id ~= nil, "Graph create response missing id")

local patched = harness.http_json({
    method = "PATCH",
    url = graph_url(graph_endpoint, "me/events/ev-001"),
    body = {
        subject = "Remote moved",
        body = {
            contentType = "text",
            content = "Updated directly through the mock Graph endpoint",
        },
        location = {
            displayName = "Remote Room B",
        },
        start = {
            dateTime = "2026-01-15T10:00:00Z",
            timeZone = "UTC",
        },
        ["end"] = {
            dateTime = "2026-01-15T10:45:00Z",
            timeZone = "UTC",
        },
        isAllDay = false,
    },
})
harness.assert(patched ~= nil, "Graph patch returned no body")

harness.http_json({
    method = "DELETE",
    url = graph_url(graph_endpoint, "me/events/ev-002"),
})

harness.clear_mock_requests(admin_endpoint)

harness.marker("SYNC_START")
local delta, delta_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(delta_err == nil, "delta start_calendar_sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "delta calendar sync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(after_err == nil, "post-delta TestQueryDbState failed")
harness.assert_eq(after.calendar_count, 2, "post-delta calendar count")
harness.assert_eq(after.calendar_event_count, 2, "post-delta event count")

local after_standup = event_by_remote_id(after.calendar_events, "ev-001")
harness.assert(after_standup ~= nil, "updated Standup missing after delta")
harness.assert_eq(after_standup.summary, "Remote moved", "updated summary")
harness.assert_eq(after_standup.location, "Remote Room B", "updated location")
harness.assert_eq(after_standup.start_time, 1768471200, "updated start")
harness.assert_eq(after_standup.end_time, 1768473900, "updated end")

local after_created = event_by_remote_id(after.calendar_events, created.id)
harness.assert(after_created ~= nil, "remote-created event missing after delta")
harness.assert_eq(after_created.summary, "Remote created", "created summary")
harness.assert_eq(after_created.location, "Remote Room", "created location")

local after_review = event_by_remote_id(after.calendar_events, "ev-002")
harness.assert(after_review == nil, "remote-deleted review still present")

local requests = harness.mock_requests(admin_endpoint)
local work_delta_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/calendars/cal-work/calendarView/delta"
)
local personal_delta_requests = harness.request_count(
    requests,
    "graph",
    "GET /v1.0/me/calendars/cal-personal/calendarView/delta"
)
harness.assert(
    work_delta_requests >= 1,
    "delta sync did not call Work calendar delta"
)
harness.assert(
    personal_delta_requests >= 1,
    "delta sync did not call Personal calendar delta"
)

harness.write_summary({
    correct = 1,
    calendar_count = after.calendar_count,
    calendar_event_count = after.calendar_event_count,
    provider_requests = #requests,
    graph_work_delta_requests = work_delta_requests,
    graph_personal_delta_requests = personal_delta_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
