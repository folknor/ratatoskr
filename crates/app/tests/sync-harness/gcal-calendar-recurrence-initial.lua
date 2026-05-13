-- description: Google Calendar initial sync imports recurrence rules from the small fixture
-- expected: pass
-- fixture: calendar-recurrence-small.toml
-- protocol: gcal
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

local function assert_rule_contains(event, needle, label)
    local rule = event.recurrence_rule or ""
    harness.assert(
        string.find(rule, needle, 1, true) ~= nil,
        label .. " recurrence rule missing " .. needle .. ": " .. rule
    )
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_gcal_calendar_recurrence_initial")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gcal-calendar-recurrence@example.test",
    display_name = "Sync Google Calendar Recurrence",
    account_name = "Sync Google Calendar Recurrence",
    provider = "gmail_api",
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
harness.assert_eq(state.calendar_count, 1, "calendar count")
harness.assert_eq(state.calendar_event_count, 3, "calendar event count")

local work = calendar_by_remote_id(state.calendars, "cal-work")
harness.assert(work ~= nil, "missing Work calendar")
harness.assert_eq(work.provider, "google", "Work calendar provider")
harness.assert_eq(work.display_name, "Work", "Work calendar name")

local weekly = event_by_remote_id(state.calendar_events, "ev-weekly")
harness.assert(weekly ~= nil, "missing weekly event")
harness.assert_eq(weekly.summary, "Team sync", "weekly summary")
harness.assert_eq(weekly.start_time, 1768813200, "weekly start_time")
harness.assert_eq(weekly.end_time, 1768815000, "weekly end_time")
assert_rule_contains(weekly, "FREQ=WEEKLY", "weekly")
assert_rule_contains(weekly, "BYDAY=MO,WE,FR", "weekly")
assert_rule_contains(weekly, "COUNT=10", "weekly")

local monthly = event_by_remote_id(state.calendar_events, "ev-monthly")
harness.assert(monthly ~= nil, "missing monthly event")
harness.assert_eq(monthly.summary, "Pay-day reminder", "monthly summary")
harness.assert_eq(monthly.start_time, 1768496400, "monthly start_time")
harness.assert_eq(monthly.end_time, 1768497300, "monthly end_time")
assert_rule_contains(monthly, "FREQ=MONTHLY", "monthly")
assert_rule_contains(monthly, "BYMONTHDAY=15", "monthly")
assert_rule_contains(monthly, "UNTIL=20261215T170000Z", "monthly")

local single = event_by_remote_id(state.calendar_events, "ev-single")
harness.assert(single ~= nil, "missing single event")
harness.assert_eq(single.summary, "Annual retreat planning", "single summary")
harness.assert(single.recurrence_rule == nil, "single event should not have recurrence")

local requests = harness.mock_requests(admin_endpoint)
local calendar_list_requests =
    harness.request_count(requests, "gcal", "GET /calendar/v3/users/me/calendarList")
local work_event_requests = harness.request_count(
    requests,
    "gcal",
    "GET /calendar/v3/calendars/cal-work/events"
)
harness.assert(calendar_list_requests >= 1, "Google Calendar sync did not list calendars")
harness.assert(work_event_requests >= 1, "Google Calendar sync did not list Work events")

harness.write_summary({
    correct = 1,
    calendar_count = state.calendar_count,
    calendar_event_count = state.calendar_event_count,
    provider_requests = #requests,
    gcal_calendar_list_requests = calendar_list_requests,
    gcal_work_event_requests = work_event_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
