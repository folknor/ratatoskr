-- description: Graph calendar actions create, update, and delete through the Service worker
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: graph
-- ceiling: 120s

local function join_url(base, suffix)
    if string.sub(base, -1) == "/" then
        return base .. suffix
    end
    return base .. "/" .. suffix
end

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

local function assert_success(completed, label)
    harness.assert(completed ~= nil, label .. " missing completion")
    harness.assert_eq(#completed.results, 1, label .. " result count")
    local result = completed.results[1].result
    harness.assert(result ~= nil, label .. " result missing")
    harness.assert_eq(result.kind, "success", label .. " result")
end

local function request_count(requests, protocol, command)
    local count = 0
    for _, request in ipairs(requests) do
        if request.protocol == protocol and request.command == command then
            count = count + 1
        end
    end
    return count
end

local function request_body(requests, command)
    for _, request in ipairs(requests) do
        if request.protocol == "graph" and request.command == command then
            if request.detail ~= nil and request.detail.body ~= nil then
                return request.detail.body
            end
        end
    end
    return nil
end

-- saehrimnir mounts test admin routes on the always-started JMAP HTTP listener.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local requests_url = join_url(admin_endpoint, "test/requests")
harness.http_delete(requests_url)

local dir = harness.data_dir("m6_calendar_actions_graph_crud")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-calendar-actions@example.test",
    display_name = "M6 Calendar Actions",
    account_name = "M6 Calendar Actions",
    provider = "graph",
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
local work = calendar_by_remote_id(initial.calendars, "cal-work")
harness.assert(work ~= nil, "missing Work calendar")
local standup = event_by_remote_id(initial.calendar_events, "ev-001")
harness.assert(standup ~= nil, "missing Standup")
local review = event_by_remote_id(initial.calendar_events, "ev-002")
harness.assert(review ~= nil, "missing Quarterly review")

harness.http_delete(requests_url)

local create_input = {
    title = "Harness created",
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
    title = "Standup moved",
    description = "Moved by harness mutation coverage",
    location = "Conf Room B",
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

local final_standup = event_by_remote_id(final.calendar_events, "ev-001")
harness.assert(final_standup ~= nil, "updated Standup missing")
harness.assert_eq(final_standup.summary, "Standup moved", "updated summary")
harness.assert_eq(final_standup.location, "Conf Room B", "updated location")
harness.assert_eq(final_standup.start_time, 1768471200, "updated start")
harness.assert_eq(final_standup.end_time, 1768473900, "updated end")

local final_created = event_by_remote_id(final.calendar_events, "mock-event-create")
harness.assert(final_created ~= nil, "created event missing")
harness.assert_eq(final_created.summary, "Harness created", "created summary")
harness.assert_eq(final_created.location, "Focus Room", "created location")

local final_review = event_by_remote_id(final.calendar_events, "ev-002")
harness.assert(final_review == nil, "deleted review still present")

local requests = harness.http_get(requests_url)
local create_body = request_body(
    requests,
    "POST /v1.0/me/calendars/cal-work/events"
)
harness.assert(create_body ~= nil, "missing create request body")
harness.assert_eq(create_body.subject, "Harness created", "create subject")
harness.assert_eq(create_body.location.displayName, "Focus Room", "create location")
harness.assert_eq(
    create_body.start.dateTime,
    "2026-02-03T10:00:00Z",
    "create start"
)

local patch_body = request_body(requests, "PATCH /v1.0/me/events/ev-001")
harness.assert(patch_body ~= nil, "missing patch request body")
harness.assert_eq(patch_body.subject, "Standup moved", "patch subject")
harness.assert_eq(patch_body.location.displayName, "Conf Room B", "patch location")
harness.assert_eq(
    patch_body.start.dateTime,
    "2026-01-15T10:00:00Z",
    "patch start"
)
harness.assert(
    request_count(requests, "graph", "DELETE /v1.0/me/events/ev-002") >= 1,
    "missing delete request"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
