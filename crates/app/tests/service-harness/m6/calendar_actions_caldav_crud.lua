-- description: CalDAV calendar actions create, update, and delete through the Service worker
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

local function event_by_summary(events, summary)
    for _, event in ipairs(events) do
        if event.summary == summary then
            return event
        end
    end
    return nil
end

local function assert_success(completed, label)
    harness.assert(completed ~= nil, label .. " missing completion")
    harness.assert_eq(#completed.results, 1, label .. " result count")
    local result = completed.results[1].result
    harness.assert(result ~= nil, label .. " result missing")
    harness.assert_eq(result.kind, "success", label .. " result")
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("m6_calendar_actions_caldav_crud")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-calendar-actions-caldav@example.test",
    display_name = "M6 Calendar Actions CalDAV",
    account_name = "M6 Calendar Actions CalDAV",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local synced, sync_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.assert(sync_err == nil, "start_calendar_sync failed")
harness.assert_eq(synced.result, "completed", synced.error or "calendar sync result")

local initial, initial_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(initial_err == nil, "initial TestQueryDbState failed")
local work = calendar_by_remote_suffix(
    initial.calendars,
    "/calendars/account-1/cal-work/"
)
harness.assert(work ~= nil, "missing Work calendar")
local standup = event_by_google_event_id(initial.calendar_events, "caldav:ev-001")
harness.assert(standup ~= nil, "missing Standup")
local review = event_by_google_event_id(initial.calendar_events, "caldav:ev-002")
harness.assert(review ~= nil, "missing Quarterly review")

harness.clear_mock_requests(admin_endpoint)

local create_input = {
    title = "Harness CalDAV created",
    description = "Created through cal_action.execute_plan",
    location = "Focus Room",
    start_time = 1770112800,
    end_time = 1770114600,
    is_all_day = false,
    timezone = "UTC",
}
local created, create_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = account.account_id,
            operation = "CreateEvent",
            calendar_id = work.id,
            input = create_input,
        },
    },
}, 30)
harness.assert(create_err == nil, "create calendar action failed")
assert_success(created, "create")

local update_input = {
    title = "CalDAV Standup moved",
    description = "Moved by harness mutation coverage",
    location = "Conf Room C",
    start_time = 1768471200,
    end_time = 1768473900,
    is_all_day = false,
    timezone = "UTC",
}
local updated, update_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = account.account_id,
            operation = "UpdateEvent",
            event_id = standup.id,
            input = update_input,
        },
    },
}, 30)
harness.assert(update_err == nil, "update calendar action failed")
assert_success(updated, "update")

local deleted, delete_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = account.account_id,
            operation = "DeleteEvent",
            event_id = review.id,
        },
    },
}, 30)
harness.assert(delete_err == nil, "delete calendar action failed")
assert_success(deleted, "delete")

local final, final_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(final_err == nil, "final TestQueryDbState failed")
harness.assert_eq(final.calendar_count, 2, "calendar count")
harness.assert_eq(final.calendar_event_count, 2, "calendar event count")

local final_standup = event_by_google_event_id(
    final.calendar_events,
    "caldav:ev-001"
)
harness.assert(final_standup ~= nil, "updated Standup missing")
harness.assert_eq(final_standup.summary, "CalDAV Standup moved", "updated summary")
harness.assert_eq(final_standup.location, "Conf Room C", "updated location")
harness.assert_eq(final_standup.start_time, 1768471200, "updated start")
harness.assert_eq(final_standup.end_time, 1768473900, "updated end")

local final_created = event_by_summary(final.calendar_events, "Harness CalDAV created")
harness.assert(final_created ~= nil, "created event missing")
harness.assert_eq(final_created.location, "Focus Room", "created location")
harness.assert(
    string.find(final_created.remote_event_id or "", "/calendars/account-1/cal-work/", 1, true) ~= nil,
    "created remote href missing Work calendar"
)

local final_review = event_by_google_event_id(final.calendar_events, "caldav:ev-002")
harness.assert(final_review == nil, "deleted review still present")

local requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(requests, "caldav", "GET /calendars/account-1/cal-work/ev-001.ics") >= 1,
    "missing CalDAV GET before update"
)
harness.assert(
    harness.request_count(requests, "caldav", "PUT /calendars/account-1/cal-work/ev-001.ics") >= 1,
    "missing CalDAV PUT update"
)
harness.assert(
    harness.request_count_prefix(requests, "caldav", "PUT /calendars/account-1/cal-work/") >= 2,
    "missing CalDAV PUT create/update traffic"
)
harness.assert(
    harness.request_count(requests, "caldav", "DELETE /calendars/account-1/cal-work/ev-002.ics") >= 1,
    "missing CalDAV DELETE"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
