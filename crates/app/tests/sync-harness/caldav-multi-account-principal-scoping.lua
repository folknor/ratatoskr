-- description: CalDAV principal URLs only expose the authenticated account's calendars and events
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

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_caldav_multi_account_principal_scoping")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Each ratatoskr account points at the same CalDAV endpoint but
-- authenticates as a different fixture principal via caldav_username.
-- Saehrimnir's CalDAV layer mounts /principals/{user}/ and
-- /calendars/{user}/... per the username, and 404s cross-account
-- lookups under the wrong principal URL.
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

local function run_sync(account_id, label)
    local result, sync_err = client:start_calendar_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_calendar_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

run_sync(primary.account_id, "primary")
run_sync(secondary.account_id, "secondary")

local function query(account_id, label)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        calendar_limit = 10,
    })
    harness.assert(state_err == nil, label .. " TestQueryDbState failed")
    return state
end

local primary_state = query(primary.account_id, "primary")
harness.assert_eq(primary_state.calendar_count, 1, "primary calendar count")
harness.assert_eq(primary_state.calendar_event_count, 1, "primary event count")
harness.assert(
    calendar_by_remote_suffix(
        primary_state.calendars,
        "/calendars/account-primary/cal-primary-work/"
    ) ~= nil,
    "primary missing Work calendar"
)
harness.assert(
    calendar_by_remote_suffix(
        primary_state.calendars,
        "/calendars/account-secondary/cal-secondary-personal/"
    ) == nil,
    "primary leaked secondary's Personal calendar"
)
local primary_event = event_by_google_event_id(
    primary_state.calendar_events,
    "caldav:ev-primary-001"
)
harness.assert(primary_event ~= nil, "primary missing its own event")
harness.assert_eq(primary_event.summary, "Primary standup", "primary event summary")
harness.assert(
    event_by_google_event_id(primary_state.calendar_events, "caldav:ev-secondary-001") == nil,
    "primary leaked secondary's event"
)

local secondary_state = query(secondary.account_id, "secondary")
harness.assert_eq(secondary_state.calendar_count, 1, "secondary calendar count")
harness.assert_eq(secondary_state.calendar_event_count, 1, "secondary event count")
harness.assert(
    calendar_by_remote_suffix(
        secondary_state.calendars,
        "/calendars/account-secondary/cal-secondary-personal/"
    ) ~= nil,
    "secondary missing Personal calendar"
)
harness.assert(
    calendar_by_remote_suffix(
        secondary_state.calendars,
        "/calendars/account-primary/cal-primary-work/"
    ) == nil,
    "secondary leaked primary's Work calendar"
)
local secondary_event = event_by_google_event_id(
    secondary_state.calendar_events,
    "caldav:ev-secondary-001"
)
harness.assert(secondary_event ~= nil, "secondary missing its own event")
harness.assert_eq(
    secondary_event.summary,
    "Secondary haircut",
    "secondary event summary"
)
harness.assert(
    event_by_google_event_id(secondary_state.calendar_events, "caldav:ev-primary-001") == nil,
    "secondary leaked primary's event"
)

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local primary_principal_propfind = harness.request_count(
    requests,
    "caldav",
    "PROPFIND /principals/account-primary/"
)
local secondary_principal_propfind = harness.request_count(
    requests,
    "caldav",
    "PROPFIND /principals/account-secondary/"
)
local primary_calendar_home_propfind = harness.request_count(
    requests,
    "caldav",
    "PROPFIND /calendars/account-primary/"
)
local secondary_calendar_home_propfind = harness.request_count(
    requests,
    "caldav",
    "PROPFIND /calendars/account-secondary/"
)
harness.assert(
    primary_principal_propfind >= 1,
    "primary sync did not PROPFIND its own principal URL"
)
harness.assert(
    secondary_principal_propfind >= 1,
    "secondary sync did not PROPFIND its own principal URL"
)
harness.assert(
    primary_calendar_home_propfind >= 1,
    "primary sync did not list its calendar home"
)
harness.assert(
    secondary_calendar_home_propfind >= 1,
    "secondary sync did not list its calendar home"
)

harness.write_summary({
    correct = 1,
    primary_calendar_count = primary_state.calendar_count,
    secondary_calendar_count = secondary_state.calendar_count,
    primary_event_count = primary_state.calendar_event_count,
    secondary_event_count = secondary_state.calendar_event_count,
    caldav_primary_principal_propfind = primary_principal_propfind,
    caldav_secondary_principal_propfind = secondary_principal_propfind,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
