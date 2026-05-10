-- description: CalDAV remote mutations are imported by calendar sync
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: caldav
-- ceiling: 120s

local function event_by_google_event_id(events, google_event_id)
    for _, event in ipairs(events) do
        if event.google_event_id == google_event_id then
            return event
        end
    end
    return nil
end

local function caldav_url(base, path)
    return harness.join_url(base, path)
end

local function ical_event(uid, summary, description, location, dtstart, dtend)
    return table.concat({
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//Ratatoskr Harness//CalDAV//EN",
        "BEGIN:VEVENT",
        "UID:" .. uid,
        "DTSTAMP:20260101T000000Z",
        "SUMMARY:" .. summary,
        "DESCRIPTION:" .. description,
        "LOCATION:" .. location,
        "DTSTART:" .. dtstart,
        "DTEND:" .. dtend,
        "END:VEVENT",
        "END:VCALENDAR",
    }, "\r\n")
end

local function assert_http_ok(response, label)
    harness.assert(response ~= nil, label .. " missing response")
    harness.assert(response.ok, label .. " returned status " .. tostring(response.status))
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_caldav_calendar_remote_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-caldav-calendar-delta@example.test",
    display_name = "Sync CalDAV Calendar Delta",
    account_name = "Sync CalDAV Calendar Delta",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-1",
    caldav_password = "test-password",
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
    event_by_google_event_id(initial.calendar_events, "caldav:ev-001") ~= nil,
    "missing Standup before mutation"
)
harness.assert(
    event_by_google_event_id(initial.calendar_events, "caldav:ev-002") ~= nil,
    "missing Quarterly review before mutation"
)

local created = harness.http({
    method = "PUT",
    url = caldav_url(caldav_endpoint, "calendars/account-1/cal-work/remote-created.ics"),
    content_type = "text/calendar; charset=utf-8",
    body = ical_event(
        "remote-created",
        "Remote CalDAV created",
        "Created directly through the mock CalDAV endpoint",
        "Remote Room",
        "20260210T100000Z",
        "20260210T103000Z"
    ),
})
assert_http_ok(created, "CalDAV create")

local updated = harness.http({
    method = "PUT",
    url = caldav_url(caldav_endpoint, "calendars/account-1/cal-work/ev-001.ics"),
    content_type = "text/calendar; charset=utf-8",
    body = ical_event(
        "ev-001",
        "Remote CalDAV moved",
        "Updated directly through the mock CalDAV endpoint",
        "Remote CalDAV Room",
        "20260115T100000Z",
        "20260115T104500Z"
    ),
})
assert_http_ok(updated, "CalDAV update")

local deleted = harness.http({
    method = "DELETE",
    url = caldav_url(caldav_endpoint, "calendars/account-1/cal-work/ev-002.ics"),
})
assert_http_ok(deleted, "CalDAV delete")

harness.clear_mock_requests(admin_endpoint)

local delta, delta_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.assert(delta_err == nil, "delta start_calendar_sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "delta calendar sync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(after_err == nil, "post-delta TestQueryDbState failed")
harness.assert_eq(after.calendar_count, 2, "post-delta calendar count")
harness.assert_eq(after.calendar_event_count, 2, "post-delta event count")

local after_standup = event_by_google_event_id(after.calendar_events, "caldav:ev-001")
harness.assert(after_standup ~= nil, "updated Standup missing after sync")
harness.assert_eq(after_standup.summary, "Remote CalDAV moved", "updated summary")
harness.assert_eq(after_standup.location, "Remote CalDAV Room", "updated location")
harness.assert_eq(after_standup.start_time, 1768471200, "updated start")
harness.assert_eq(after_standup.end_time, 1768473900, "updated end")

local after_created = event_by_google_event_id(
    after.calendar_events,
    "caldav:remote-created"
)
harness.assert(after_created ~= nil, "remote-created event missing after sync")
harness.assert_eq(after_created.summary, "Remote CalDAV created", "created summary")
harness.assert_eq(after_created.location, "Remote Room", "created location")

local after_review = event_by_google_event_id(after.calendar_events, "caldav:ev-002")
harness.assert(after_review == nil, "remote-deleted review still present")

local requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(requests, "caldav", "PROPFIND /calendars/account-1/cal-work/") >= 1,
    "CalDAV sync did not refresh Work ctag"
)
harness.assert(
    harness.request_count(requests, "caldav", "REPORT /calendars/account-1/cal-work/") >= 1,
    "CalDAV sync did not fetch Work changes"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
