-- description: CalDAV calendar omissions hide cached data, clear on reappearance, and reap only after seven days
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: caldav
-- ceiling: 120s

local T0 = 1768000000000
local HOUR_MS = 60 * 60 * 1000
local DAY_MS = 24 * HOUR_MS

local function calendar_by_suffix(calendars, suffix)
    for _, calendar in ipairs(calendars) do
        if string.sub(calendar.remote_id, -string.len(suffix)) == suffix then
            return calendar
        end
    end
    return nil
end

local function assert_http_ok(response, label)
    harness.assert(response ~= nil, label .. " missing response")
    harness.assert(response.ok, label .. " returned status " .. tostring(response.status))
end

local function sync(client, account_id, now_ms, label)
    local completed, err = client:start_calendar_sync({
        account_id = account_id,
        now_ms = now_ms,
    }, 30)
    harness.assert(err == nil, label .. " start_calendar_sync failed")
    harness.assert_eq(completed.result, "completed", completed.error or label)
end

local function state(client, account_id, label)
    local value, err = client:request("TestQueryDbState", {
        account_id = account_id,
        calendar_limit = 10,
    })
    harness.assert(err == nil, label .. " TestQueryDbState failed")
    return value
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")

local personal_url = harness.join_url(caldav_endpoint, "calendars/account-1/cal-personal/")
local work_url = harness.join_url(caldav_endpoint, "calendars/account-1/cal-work/")
local dir = harness.data_dir("sync_caldav_calendar_unlisted_reap")
local client, spawn_err = harness.spawn(dir)
harness.assert(spawn_err == nil, "spawn failed")
local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil and ready.ready, "boot failed")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-caldav-calendar-unlisted@example.test",
    display_name = "Sync CalDAV Calendar Unlisted",
    account_name = "Sync CalDAV Calendar Unlisted",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

sync(client, account.account_id, T0, "initial")
local initial = state(client, account.account_id, "initial")
harness.assert_eq(initial.calendar_count, 2, "initial calendar count")
harness.assert_eq(initial.calendar_event_count, 2, "initial event count")
local personal = calendar_by_suffix(initial.calendars, "/calendars/account-1/cal-personal/")
local work = calendar_by_suffix(initial.calendars, "/calendars/account-1/cal-work/")
harness.assert(personal ~= nil and work ~= nil, "fixture calendars missing")

assert_http_ok(harness.http({ method = "DELETE", url = personal_url }), "unlist Personal")
sync(client, account.account_id, T0, "stamp omission")
local stamped = state(client, account.account_id, "stamped")
local stamped_personal = calendar_by_suffix(stamped.calendars, "/calendars/account-1/cal-personal/")
harness.assert_eq(stamped.calendar_count, 2, "unlisted row must be retained")
harness.assert(stamped_personal.unlisted_since ~= nil, "omitted calendar was not stamped")
harness.assert_eq(stamped_personal.unlisted_since, T0, "stamp time")
harness.assert(
    not table.concat(stamped.sidebar_calendar_ids, ","):find(stamped_personal.id, 1, true),
    "stamped calendar leaked into production sidebar loader"
)
for _, event in ipairs(stamped.calendar_events) do
    harness.assert(
        not table.concat(stamped.view_calendar_event_ids, ","):find(event.id, 1, true)
            or event.calendar_id ~= stamped_personal.id,
        "stamped calendar event leaked into production agenda loader"
    )
end

assert_http_ok(harness.http({ method = "MKCALENDAR", url = personal_url }), "restore Personal")
sync(client, account.account_id, T0 + HOUR_MS, "reappear")
local reappeared = state(client, account.account_id, "reappeared")
local reappeared_personal = calendar_by_suffix(reappeared.calendars, "/calendars/account-1/cal-personal/")
harness.assert_eq(reappeared_personal.unlisted_since, nil, "reappearance did not clear stamp")
harness.assert(
    table.concat(reappeared.sidebar_calendar_ids, ","):find(reappeared_personal.id, 1, true),
    "reappeared calendar did not return to production sidebar loader"
)

-- Use the other original collection for the reap branch. This avoids a
-- second DELETE of the freshly recreated collection while still proving that
-- a later non-empty omission stamps and then reaps cached events.
assert_http_ok(harness.http({ method = "DELETE", url = work_url }), "unlist Work")
sync(client, account.account_id, T0 + 2 * HOUR_MS, "stamp Work")
local work_stamped = state(client, account.account_id, "stamp Work")
local stamped_work = calendar_by_suffix(work_stamped.calendars, "/calendars/account-1/cal-work/")
harness.assert_eq(stamped_work.unlisted_since, T0 + 2 * HOUR_MS, "Work stamp time")

-- Failure-does-not-reap (spec 5.7 step 5): drop the remaining collection so the
-- discovery list comes back empty at a now_ms already past the reap threshold.
-- A successful-but-empty list is treated as a suspected transient, so it must
-- neither stamp nor reap - the stamped Work row and its cached data survive.
assert_http_ok(harness.http({ method = "DELETE", url = personal_url }), "unlist Personal for empty-list arm")
sync(client, account.account_id, T0 + 8 * DAY_MS, "empty list past threshold")
local empty = state(client, account.account_id, "empty list")
harness.assert_eq(empty.calendar_count, 2, "empty list past threshold must not reap")
local empty_work = calendar_by_suffix(empty.calendars, "/calendars/account-1/cal-work/")
harness.assert(empty_work ~= nil, "empty list reaped the stamped Work row")
harness.assert_eq(empty_work.unlisted_since, T0 + 2 * HOUR_MS, "empty list must not advance the stamp")

-- Restore Personal so the next list is non-empty again; only now, with the
-- provider affirmatively listing another collection while omitting Work, does
-- the expired Work row reap.
assert_http_ok(harness.http({ method = "MKCALENDAR", url = personal_url }), "restore Personal for reap arm")

-- Seed one event onto the restored Personal collection. The graph-calendar-small
-- fixture puts both events on cal-work and leaves cal-personal deliberately
-- empty (the zero-events branch), so a bare total-event count after the reap
-- cannot tell "reap dropped only the omitted Work events" apart from "reap
-- dropped everything". Giving the surviving, listed collection an event of its
-- own turns the reap into a scoping proof: Work's two events (with attendees)
-- must all vanish while this one Personal event, keyed to the Personal
-- calendar, must remain.
local personal_event_url = personal_url .. "ev-personal-reap.ics"
local personal_event_ics = table.concat({
    "BEGIN:VCALENDAR",
    "VERSION:2.0",
    "PRODID:-//ratatoskr//harness//EN",
    "BEGIN:VEVENT",
    "UID:ev-personal-reap",
    "SUMMARY:Personal survives the reap",
    "DTSTART:20260120T090000Z",
    "DTEND:20260120T093000Z",
    "END:VEVENT",
    "END:VCALENDAR",
}, "\r\n") .. "\r\n"
assert_http_ok(
    harness.http({
        method = "PUT",
        url = personal_event_url,
        body = personal_event_ics,
        content_type = "text/calendar; charset=utf-8",
    }),
    "seed Personal event"
)

sync(client, account.account_id, T0 + 8 * DAY_MS, "reap")
local reaped = state(client, account.account_id, "reaped")
harness.assert_eq(reaped.calendar_count, 1, "expired omitted calendar was not reaped")
harness.assert(calendar_by_suffix(reaped.calendars, "/calendars/account-1/cal-work/") == nil,
    "reaped Work row remains")
local reaped_personal = calendar_by_suffix(reaped.calendars, "/calendars/account-1/cal-personal/")
harness.assert(reaped_personal ~= nil, "listed Personal calendar was reaped")
-- Scoping proof: exactly the one Personal event survives, and it is keyed to
-- the still-listed Personal calendar - Work's two events were dropped by the
-- reap, not merely reduced in count.
harness.assert_eq(reaped.calendar_event_count, 1, "reap must drop only the omitted calendar's events")
harness.assert_eq(#reaped.calendar_events, 1, "reap left an unexpected number of event rows")
harness.assert_eq(reaped.calendar_events[1].calendar_id, reaped_personal.id,
    "surviving event does not belong to the listed Personal calendar")
harness.assert_eq(reaped.calendar_attendee_count, 0, "reap leaked attendees")
harness.assert_eq(reaped.calendar_reminder_count, 0, "reap leaked reminders")

harness.write_summary({
    correct = 1,
    calendar_count = reaped.calendar_count,
    calendar_event_count = reaped.calendar_event_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
