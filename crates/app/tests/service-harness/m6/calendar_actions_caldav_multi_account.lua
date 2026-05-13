-- description: CalDAV CRUD through the secondary principal stays under /calendars/account-secondary/ and never touches primary
-- expected: pass
-- fixture: multi-account-calendar-small.toml
-- protocol: caldav
-- ceiling: 120s

local function calendar_by_remote_suffix(calendars, suffix)
    for _, calendar in ipairs(calendars) do
        if string.sub(calendar.remote_id or "", -string.len(suffix)) == suffix then
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

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("m6_calendar_actions_caldav_multi_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Seed both accounts; mutations only happen on secondary.
local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "CalDAV Primary",
    account_name = "CalDAV Primary",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-primary",
    caldav_password = "test-password",
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "CalDAV Secondary",
    account_name = "CalDAV Secondary",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-secondary",
    caldav_password = "test-password",
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

-- Prime both with an initial sync so the local DB has the fixture
-- events and calendars to mutate.
local function run_sync(account_id, label)
    local result, sync_err = client:start_calendar_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_calendar_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

run_sync(primary.account_id, "primary initial")
run_sync(secondary.account_id, "secondary initial")

local secondary_state, secondary_state_err = client:request("TestQueryDbState", {
    account_id = secondary.account_id,
    calendar_limit = 10,
})
harness.assert(secondary_state_err == nil, "secondary TestQueryDbState failed")
local secondary_personal = calendar_by_remote_suffix(
    secondary_state.calendars,
    "/calendars/account-secondary/cal-secondary-personal/"
)
harness.assert(secondary_personal ~= nil, "missing secondary Personal calendar")
local secondary_haircut = event_by_google_event_id(
    secondary_state.calendar_events,
    "caldav:ev-secondary-001"
)
harness.assert(secondary_haircut ~= nil, "missing secondary fixture event")

harness.clear_mock_requests(admin_endpoint)

-- 1. Create a brand-new event through the secondary account's
--    Personal calendar.
local created, create_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = secondary.account_id,
            operation = "CreateEvent",
            calendar_id = secondary_personal.id,
            input = {
                title = "Secondary harness create",
                description = "Created on the secondary principal only",
                location = "Secondary office",
                start_time = 1770112800,
                end_time = 1770114600,
                is_all_day = false,
                timezone = "UTC",
            },
        },
    },
}, 30)
harness.assert(create_err == nil, "create on secondary failed")
assert_success(created, "create")

-- 2. Update the fixture event on the secondary calendar.
local updated, update_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = secondary.account_id,
            operation = "UpdateEvent",
            event_id = secondary_haircut.id,
            input = {
                title = "Secondary haircut rescheduled",
                description = "Updated on the secondary principal only",
                location = "Secondary salon B",
                start_time = 1769608800,
                end_time = 1769611500,
                is_all_day = false,
                timezone = "UTC",
            },
        },
    },
}, 30)
harness.assert(update_err == nil, "update on secondary failed")
assert_success(updated, "update")

-- 3. Sync the primary to prove the secondary's mutations never appear
--    on the primary, then sync the secondary so we can read back the
--    final state.
run_sync(primary.account_id, "primary post-mutation")
run_sync(secondary.account_id, "secondary post-mutation")

local final_secondary, final_secondary_err = client:request("TestQueryDbState", {
    account_id = secondary.account_id,
    calendar_limit = 10,
})
harness.assert(final_secondary_err == nil, "final secondary TestQueryDbState failed")
harness.assert_eq(final_secondary.calendar_event_count, 2, "secondary event count after create")

local final_created = event_by_summary(
    final_secondary.calendar_events,
    "Secondary harness create"
)
harness.assert(final_created ~= nil, "created event missing from secondary")
harness.assert(
    string.find(
        final_created.remote_event_id or "",
        "/calendars/account-secondary/cal-secondary-personal/",
        1,
        true
    ) ~= nil,
    "created event href is not under /calendars/account-secondary/..."
)
local final_haircut = event_by_google_event_id(
    final_secondary.calendar_events,
    "caldav:ev-secondary-001"
)
harness.assert(final_haircut ~= nil, "updated event missing from secondary")
harness.assert_eq(
    final_haircut.summary,
    "Secondary haircut rescheduled",
    "secondary fixture event was not updated"
)

local final_primary, final_primary_err = client:request("TestQueryDbState", {
    account_id = primary.account_id,
    calendar_limit = 10,
})
harness.assert(final_primary_err == nil, "final primary TestQueryDbState failed")
harness.assert_eq(
    final_primary.calendar_event_count,
    1,
    "primary picked up secondary's CRUD - cross-account leakage"
)
harness.assert(
    event_by_summary(final_primary.calendar_events, "Secondary harness create") == nil,
    "secondary's created event leaked into primary"
)
harness.assert(
    event_by_summary(final_primary.calendar_events, "Secondary haircut rescheduled") == nil,
    "secondary's updated event leaked into primary"
)

local requests = harness.mock_requests(admin_endpoint, { stable = true })

-- Every CalDAV write the secondary issued must address /calendars/account-secondary/...
local secondary_put = harness.request_count_prefix(
    requests,
    "caldav",
    "PUT /calendars/account-secondary/cal-secondary-personal/"
)
harness.assert(
    secondary_put >= 2,
    "expected at least one PUT for create and one for update under /calendars/account-secondary/..."
)
local primary_put = harness.request_count_prefix(
    requests,
    "caldav",
    "PUT /calendars/account-primary/"
)
harness.assert_eq(
    primary_put,
    0,
    "secondary mutation wrote to /calendars/account-primary/..."
)

-- 4. Delete the secondary's fixture event; assert the DELETE again
--    only addresses the secondary's calendar home, and the row is gone.
harness.clear_mock_requests(admin_endpoint)
local deleted, delete_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = secondary.account_id,
            operation = "DeleteEvent",
            event_id = secondary_haircut.id,
        },
    },
}, 30)
harness.assert(delete_err == nil, "delete on secondary failed")
assert_success(deleted, "delete")

run_sync(secondary.account_id, "secondary post-delete")

local after_delete, after_delete_err = client:request("TestQueryDbState", {
    account_id = secondary.account_id,
    calendar_limit = 10,
})
harness.assert(after_delete_err == nil, "after-delete TestQueryDbState failed")
harness.assert(
    event_by_google_event_id(after_delete.calendar_events, "caldav:ev-secondary-001") == nil,
    "deleted event still present on secondary"
)

local delete_requests = harness.mock_requests(admin_endpoint, { stable = true })
local secondary_delete = harness.request_count_prefix(
    delete_requests,
    "caldav",
    "DELETE /calendars/account-secondary/cal-secondary-personal/"
)
harness.assert(secondary_delete >= 1, "missing DELETE under /calendars/account-secondary/...")
local primary_delete = harness.request_count_prefix(
    delete_requests,
    "caldav",
    "DELETE /calendars/account-primary/"
)
harness.assert_eq(primary_delete, 0, "DELETE leaked to /calendars/account-primary/...")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
