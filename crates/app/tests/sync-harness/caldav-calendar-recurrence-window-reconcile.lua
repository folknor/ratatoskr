-- description: FORK 6 - windowed reconcile preserves out-of-window recurring masters and propagates a real delete
-- expected: pass
-- fixture: calendar-recurrence-window.toml
-- protocol: caldav
-- ceiling: 120s
--
-- PENDING FIXTURE (B7a item G / FORK 6): this gate needs a saehrimnir CalDAV
-- fixture `calendar-recurrence-window.toml` that does NOT exist yet. It must be
-- authored as a saehrimnir side-quest (this repo cannot edit saehrimnir). The
-- occurrence-aware CalDAV mock is already in place (a recurring event matches a
-- range iff DTSTART < range_end and its RRULE UNTIL, if any, is not before
-- range_start), so both halves below become exercisable once the fixture lands.
--
-- REQUIRED FIXTURE SHAPE (see the B7a landing report item-G spec for the exact
-- DTSTART/RRULE values; dates are chosen relative to the [-1y,+2y] active window
-- and the [-5y,-1y) backfill window):
--   Account: account-1 (caldav), one calendar collection `cal-history`
--     (remote path suffix `/calendars/account-1/cal-history/`).
--   Event A `ev-ended-series`: STRUCTURED recurring master (recurrence_rule
--     field SET, not only in raw_ical) whose occurrences ALL fall inside the
--     backfill window [now-5y, now-1y) and END before the active window - e.g.
--     DTSTART ~3.5y ago, `FREQ=WEEKLY;UNTIL=<~3y ago>`. It is reachable by the
--     backfill range pull but NOT by the active-window pull, so it is
--     absent-from-seen on every active reconcile and MUST be PRESERVED.
--   Event B `ev-active-old`: STRUCTURED recurring master started ~3.5y ago,
--     UNBOUNDED (no UNTIL/COUNT) or long COUNT, so its occurrences reach into
--     the active window. It is returned by the active-window pull (still live),
--     and is the resource REMOVED below to exercise a real delete.
--
-- This script drives the ratatoskr side end-to-end; only the fixture + the
-- assertions' exact google_event_ids depend on the side-quest.

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

local function assert_http_ok(response, label)
    harness.assert(response ~= nil, label .. " missing response")
    harness.assert(response.ok, label .. " returned status " .. tostring(response.status))
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local caldav_endpoint = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(caldav_endpoint ~= nil, "RATATOSKR_TEST_CALDAV_ENDPOINT missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_caldav_calendar_recurrence_window_reconcile")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-caldav-calendar-window@example.test",
    display_name = "Sync CalDAV Calendar Window",
    account_name = "Sync CalDAV Calendar Window",
    provider = "caldav",
    caldav_url = caldav_endpoint,
    caldav_username = "account-1",
    caldav_password = "test-password",
})
harness.assert(account_err == nil, "TestSeedAccount failed")

-- First sync: active-window pull + windowed reconcile + one-time history
-- backfill. The backfill pull [-5y,-1y) reaches the ended series; the
-- active-window pull reaches the still-live series.
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
-- Both series are cached after the first run: the ended one via the history
-- backfill, the live one via the active-window pull.
harness.assert(
    event_by_google_event_id(initial.calendar_events, "caldav:ev-ended-series") ~= nil,
    "ended out-of-window series must be backfilled and cached after first sync"
)
harness.assert(
    event_by_google_event_id(initial.calendar_events, "caldav:ev-active-old") ~= nil,
    "active out-of-window series must be cached after first sync"
)

-- Remove the still-live series from the mock so the next active-window pull no
-- longer returns it (a genuine remote delete of a recurring master with
-- in-window occurrences).
local deleted = harness.http({
    method = "DELETE",
    url = caldav_url(caldav_endpoint, "calendars/account-1/cal-history/ev-active-old.ics"),
})
assert_http_ok(deleted, "CalDAV delete of active series")

harness.clear_mock_requests(admin_endpoint)

harness.marker("SYNC_START")
local second, second_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.marker("SYNC_END")
harness.assert(second_err == nil, "second start_calendar_sync failed")
harness.assert_eq(second.result, "completed", second.error or "second calendar sync result")

local after, after_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(after_err == nil, "post-delta TestQueryDbState failed")

-- PRESERVATION half (FORK 6): the ended out-of-window series is absent from the
-- active-window seen set (its occurrences are all before the window) AND the
-- backfill no longer re-fetches it (history_backfilled_at is stamped), yet it
-- MUST NOT be reaped - its RRULE has no occurrence intersecting the active
-- window, so should_delete_absent_candidate preserves it.
harness.assert(
    event_by_google_event_id(after.calendar_events, "caldav:ev-ended-series") ~= nil,
    "ended out-of-window recurring master was wrongly deleted (FORK 6 preservation regression)"
)

-- PROPAGATION half (FORK 6): the removed live series IS absent from the pull and
-- DOES have occurrences intersecting the active window, so it is a genuine
-- remote delete and must be reaped.
harness.assert(
    event_by_google_event_id(after.calendar_events, "caldav:ev-active-old") == nil,
    "remotely-deleted live recurring series still present (delete not propagated)"
)

-- CONTROL: the in-window anchor event keeps the active-window pull non-empty (so
-- the O16 empty-pull guard does not short-circuit the reconcile) and must itself
-- survive - proving the reconcile RAN and did not over-delete a live in-window
-- event while reaping ev-active-old.
harness.assert(
    event_by_google_event_id(after.calendar_events, "caldav:ev-anchor") ~= nil,
    "in-window anchor event was wrongly deleted (reconcile over-deleted a live event)"
)

local requests = harness.mock_requests(admin_endpoint)
local history_report_requests =
    harness.request_count(requests, "caldav", "REPORT /calendars/account-1/cal-history/")
harness.assert(history_report_requests >= 1, "CalDAV sync did not fetch history calendar")

harness.write_summary({
    correct = 1,
    calendar_event_count = after.calendar_event_count,
    provider_requests = #requests,
    caldav_history_report_requests = history_report_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
