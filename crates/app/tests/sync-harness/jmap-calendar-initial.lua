-- description: JMAP calendar initial sync imports the small calendar fixture
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: jmap
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

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_calendar_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-calendar@example.test",
    display_name = "Sync JMAP Calendar",
    account_name = "Sync JMAP Calendar",
    provider = "jmap",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

harness.marker("SYNC_START")
local completed, sync_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(sync_err == nil, "start_calendar_sync failed")
harness.assert_eq(completed.result, "completed", completed.error or "calendar sync result")

local state, state_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(state_err == nil, "TestQueryDbState failed")
harness.assert_eq(state.calendar_count, 2, "calendar count")
harness.assert_eq(state.calendar_event_count, 2, "calendar event count")

local work = calendar_by_remote_id(state.calendars, "cal-work")
harness.assert(work ~= nil, "missing Work calendar")
harness.assert_eq(work.provider, "jmap", "Work calendar provider")
harness.assert_eq(work.display_name, "Work", "Work calendar name")
harness.assert(work.is_primary, "Work calendar should be primary")
harness.assert(work.is_visible, "Work calendar should be visible")
harness.assert(work.can_edit, "Work calendar should be editable")

local personal = calendar_by_remote_id(state.calendars, "cal-personal")
harness.assert(personal ~= nil, "missing Personal calendar")
harness.assert_eq(personal.display_name, "Personal", "Personal calendar name")
harness.assert(not personal.is_primary, "Personal calendar should not be primary")

local standup = event_by_remote_id(state.calendar_events, "ev-001")
harness.assert(standup ~= nil, "missing Standup event")
harness.assert_eq(standup.summary, "Standup", "Standup summary")
harness.assert_eq(standup.location, "Conf Room A", "Standup location")
harness.assert_eq(standup.start_time, 1768467600, "Standup start_time")
harness.assert_eq(standup.end_time, 1768468500, "Standup end_time")
harness.assert_eq(standup.status, "confirmed", "Standup status")
harness.assert_eq(
    standup.organizer_email,
    "alice@example.com",
    "Standup organizer"
)
harness.assert(
    string.find(standup.attendees_json or "", "bob@example.com", 1, true) ~= nil,
    "Standup attendees missing Bob"
)
harness.assert(
    string.find(standup.attendees_json or "", "carol@example.com", 1, true) ~= nil,
    "Standup attendees missing Carol"
)

local review = event_by_remote_id(state.calendar_events, "ev-002")
harness.assert(review ~= nil, "missing Quarterly review event")
harness.assert_eq(review.summary, "Quarterly review", "review summary")
harness.assert_eq(review.start_time, 1769954400, "review start_time")
harness.assert_eq(review.end_time, 1769958000, "review end_time")
harness.assert(review.attendees_json == nil, "empty attendees should stay nil")

local requests = harness.mock_requests(admin_endpoint)
local calendar_get_requests = harness.request_count(requests, "jmap", "Calendar/get")
local event_get_requests = harness.request_count(requests, "jmap", "CalendarEvent/get")
harness.assert(calendar_get_requests >= 1, "JMAP sync did not call Calendar/get")
harness.assert(event_get_requests >= 1, "JMAP sync did not call CalendarEvent/get")

harness.write_summary({
    correct = 1,
    calendar_count = state.calendar_count,
    calendar_event_count = state.calendar_event_count,
    provider_requests = #requests,
    jmap_calendar_get_requests = calendar_get_requests,
    jmap_calendar_event_get_requests = event_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
