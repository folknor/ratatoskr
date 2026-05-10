-- description: JMAP Calendar remote mutations are imported by calendar sync
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: jmap
-- ceiling: 120s

local function event_by_remote_id(events, remote_id)
    for _, event in ipairs(events) do
        if event.remote_event_id == remote_id then
            return event
        end
    end
    return nil
end

local function jmap_request(endpoint, method_name, args)
    return harness.http_json({
        method = "POST",
        url = harness.join_url(endpoint, "jmap/api"),
        body = {
            using = {
                "urn:ietf:params:jmap:core",
                "urn:ietf:params:jmap:calendars",
            },
            methodCalls = {
                {
                    [1] = method_name,
                    [2] = args,
                    [3] = "c0",
                },
            },
        },
    })
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_jmap_calendar_remote_delta")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-calendar-delta@example.test",
    display_name = "Sync JMAP Calendar Delta",
    account_name = "Sync JMAP Calendar Delta",
    provider = "jmap",
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

local mutation = jmap_request(admin_endpoint, "CalendarEvent/set", {
    accountId = "account-1",
    create = {
        c1 = {
            ["@type"] = "Event",
            calendarIds = {
                ["cal-work"] = true,
            },
            title = "Remote JMAP created",
            description = "Created directly through the mock JMAP Calendar endpoint",
            start = "2026-02-10T10:00:00",
            duration = "PT30M",
            timeZone = "UTC",
            locations = {
                loc1 = {
                    ["@type"] = "Location",
                    name = "Remote Room",
                },
            },
        },
    },
    update = {
        ["ev-001"] = {
            title = "Remote JMAP moved",
            description = "Updated directly through the mock JMAP Calendar endpoint",
            start = "2026-01-15T10:00:00",
            duration = "PT45M",
            locations = {
                loc1 = {
                    ["@type"] = "Location",
                    name = "Remote JMAP Room",
                },
            },
        },
    },
    destroy = {
        "ev-002",
    },
})
harness.assert(mutation ~= nil, "JMAP CalendarEvent/set returned no body")
harness.assert_eq(
    mutation.methodResponses[1][1],
    "CalendarEvent/set",
    "unexpected JMAP response method"
)
local created_id = mutation.methodResponses[1][2].created.c1.id
harness.assert(created_id ~= nil, "JMAP CalendarEvent/set create missing id")

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
harness.assert_eq(after_standup.summary, "Remote JMAP moved", "updated summary")
harness.assert_eq(after_standup.location, "Remote JMAP Room", "updated location")
harness.assert_eq(after_standup.start_time, 1768471200, "updated start")
harness.assert_eq(after_standup.end_time, 1768473900, "updated end")

local after_created = event_by_remote_id(after.calendar_events, created_id)
harness.assert(after_created ~= nil, "remote-created event missing after delta")
harness.assert_eq(after_created.summary, "Remote JMAP created", "created summary")
harness.assert_eq(after_created.location, "Remote Room", "created location")

local after_review = event_by_remote_id(after.calendar_events, "ev-002")
harness.assert(after_review == nil, "remote-deleted review still present")

local requests = harness.mock_requests(admin_endpoint)
local event_changes_requests = harness.request_count(requests, "jmap", "CalendarEvent/changes")
local event_get_requests = harness.request_count(requests, "jmap", "CalendarEvent/get")
harness.assert(event_changes_requests >= 1, "delta sync did not call CalendarEvent/changes")
harness.assert(event_get_requests >= 1, "delta sync did not call CalendarEvent/get")

harness.write_summary({
    correct = 1,
    calendar_count = after.calendar_count,
    calendar_event_count = after.calendar_event_count,
    provider_requests = #requests,
    jmap_calendar_event_changes_requests = event_changes_requests,
    jmap_calendar_event_get_requests = event_get_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
