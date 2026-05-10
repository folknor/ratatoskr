-- description: CalDAV calendar initial sync imports the small fixture
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: caldav
-- ceiling: 120s

local function calendar_by_remote_suffix(calendars, suffix)
    for _, calendar in ipairs(calendars) do
        if string.sub(calendar.remote_id, -string.len(suffix)) == suffix then
            return calendar
        end
    end
    return nil
end

local function event_by_google_event_id(events, google_event_id)
    for _, event in ipairs(events) do
        if event.google_event_id == google_event_id then
            return event
        end
    end
    return nil
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_caldav_calendar_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-caldav-calendar@example.test",
    display_name = "Sync CalDAV Calendar",
    account_name = "Sync CalDAV Calendar",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local completed, sync_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_calendar_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "calendar sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.calendar_count, 2, "calendar count")
harness.assert_eq(state.calendar_event_count, 2, "calendar event count")

local work = calendar_by_remote_suffix(state.calendars, "/calendars/account-1/cal-work/")
harness.assert(work ~= nil, "missing Work calendar")
harness.assert_eq(work.provider, "caldav", "Work calendar provider")
harness.assert_eq(work.display_name, "Work", "Work calendar name")
harness.assert(work.is_visible, "Work calendar should be visible")
harness.assert(work.can_edit, "Work calendar should be editable")

local personal = calendar_by_remote_suffix(
    state.calendars,
    "/calendars/account-1/cal-personal/"
)
harness.assert(personal ~= nil, "missing Personal calendar")
harness.assert_eq(personal.display_name, "Personal", "Personal calendar name")

local standup = event_by_google_event_id(state.calendar_events, "caldav:ev-001")
harness.assert(standup ~= nil, "missing Standup event")
harness.assert_eq(standup.summary, "Standup", "Standup summary")
harness.assert_eq(standup.title, "Standup", "Standup title")
harness.assert_eq(standup.location, "Conf Room A", "Standup location")
harness.assert_eq(standup.start_time, 1768467600, "Standup start_time")
harness.assert_eq(standup.end_time, 1768468500, "Standup end_time")
harness.assert(
    string.find(standup.remote_event_id or "", "/calendars/account-1/cal-work/ev-001.ics", 1, true) ~= nil,
    "Standup remote href missing fixture event id"
)
harness.assert(
    string.find(standup.attendees_json or "", "bob@example.com", 1, true) ~= nil,
    "Standup attendees missing Bob"
)
harness.assert(
    string.find(standup.attendees_json or "", "carol@example.com", 1, true) ~= nil,
    "Standup attendees missing Carol"
)

local review = event_by_google_event_id(state.calendar_events, "caldav:ev-002")
harness.assert(review ~= nil, "missing Quarterly review event")
harness.assert_eq(review.summary, "Quarterly review", "review summary")
harness.assert_eq(review.start_time, 1769954400, "review start_time")
harness.assert_eq(review.end_time, 1769958000, "review end_time")
harness.assert(review.attendees_json == nil, "empty attendees should stay nil")

local requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(requests, "caldav", "PROPFIND /") >= 1,
    "CalDAV sync did not discover principal from root"
)
harness.assert(
    harness.request_count(requests, "caldav", "PROPFIND /principals/account-1/") >= 1,
    "CalDAV sync did not discover calendar home"
)
harness.assert(
    harness.request_count(requests, "caldav", "PROPFIND /calendars/account-1/") >= 1,
    "CalDAV sync did not list calendars"
)
harness.assert(
    harness.request_count(requests, "caldav", "PROPFIND /calendars/account-1/cal-personal/") >= 1,
    "CalDAV sync did not inspect Personal events"
)
-- Personal is empty, so the CalDAV client lists it but has no event
-- hrefs to fetch via calendar-multiget REPORT.
harness.assert(
    harness.request_count(requests, "caldav", "REPORT /calendars/account-1/cal-work/") >= 1,
    "CalDAV sync did not fetch Work events"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
