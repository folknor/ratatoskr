-- description: Google Calendar remote mutations are imported by calendar sync
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: gcal
-- ceiling: 120s

local function event_by_remote_id(events, remote_id)
    for _, event in ipairs(events) do
        if event.remote_event_id == remote_id then
            return event
        end
    end
    return nil
end

local function gcal_url(base, suffix)
    return harness.join_url(base, suffix)
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local gcal_endpoint = harness.env("RATATOSKR_TEST_GCAL_ENDPOINT")
harness.assert(gcal_endpoint ~= nil, "RATATOSKR_TEST_GCAL_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_gcal_calendar_remote_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gcal-calendar-delta@example.test",
    display_name = "Sync Google Calendar Delta",
    account_name = "Sync Google Calendar Delta",
    provider = "gmail_api",
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
harness.assert(
    event_by_remote_id(initial.calendar_events, "ev-001") ~= nil,
    "missing Standup before mutation"
)
harness.assert(
    event_by_remote_id(initial.calendar_events, "ev-002") ~= nil,
    "missing Quarterly review before mutation"
)

local created = harness.http_json({
    method = "POST",
    url = gcal_url(gcal_endpoint, "calendar/v3/calendars/cal-work/events"),
    body = {
        summary = "Remote Google created",
        description = "Created directly through the mock Google Calendar endpoint",
        location = "Remote Room",
        start = {
            dateTime = "2026-02-10T10:00:00Z",
            timeZone = "UTC",
        },
        ["end"] = {
            dateTime = "2026-02-10T10:30:00Z",
            timeZone = "UTC",
        },
    },
})
harness.assert(created ~= nil, "Google Calendar create returned no body")
harness.assert(created.id ~= nil, "Google Calendar create response missing id")

local patched = harness.http_json({
    method = "PATCH",
    url = gcal_url(gcal_endpoint, "calendar/v3/calendars/cal-work/events/ev-001"),
    body = {
        summary = "Remote Google moved",
        description = "Updated directly through the mock Google Calendar endpoint",
        location = "Remote Google Room",
        start = {
            dateTime = "2026-01-15T10:00:00Z",
            timeZone = "UTC",
        },
        ["end"] = {
            dateTime = "2026-01-15T10:45:00Z",
            timeZone = "UTC",
        },
    },
})
harness.assert(patched ~= nil, "Google Calendar patch returned no body")

harness.http_json({
    method = "DELETE",
    url = gcal_url(gcal_endpoint, "calendar/v3/calendars/cal-work/events/ev-002"),
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
harness.assert_eq(after_standup.summary, "Remote Google moved", "updated summary")
harness.assert_eq(after_standup.location, "Remote Google Room", "updated location")
harness.assert_eq(after_standup.start_time, 1768471200, "updated start")
harness.assert_eq(after_standup.end_time, 1768473900, "updated end")

local after_created = event_by_remote_id(after.calendar_events, created.id)
harness.assert(after_created ~= nil, "remote-created event missing after delta")
harness.assert_eq(after_created.summary, "Remote Google created", "created summary")
harness.assert_eq(after_created.location, "Remote Room", "created location")

local after_review = event_by_remote_id(after.calendar_events, "ev-002")
harness.assert(after_review == nil, "remote-deleted review still present")

local requests = harness.mock_requests(admin_endpoint)
local work_event_requests = harness.request_count(
    requests,
    "gcal",
    "GET /calendar/v3/calendars/cal-work/events"
)
local personal_event_requests = harness.request_count(
    requests,
    "gcal",
    "GET /calendar/v3/calendars/cal-personal/events"
)
harness.assert(work_event_requests >= 1, "delta sync did not call Work events")
harness.assert(personal_event_requests >= 1, "delta sync did not call Personal events")

harness.write_summary({
    correct = 1,
    calendar_count = after.calendar_count,
    calendar_event_count = after.calendar_event_count,
    provider_requests = #requests,
    gcal_work_event_requests = work_event_requests,
    gcal_personal_event_requests = personal_event_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
