-- description: Google Calendar RSVP (Accept) writeback round-trip
-- expected: pass
-- fixture: calendar-rsvp-small.toml
-- protocol: gcal
-- ceiling: 120s

-- B7b section-6.3 RSVP-writeback gate. Syncs a meeting invite in which
-- the authed user (the primary account, whose Gmail-profile
-- `emailAddress` the mock derives as `account.name` = test@example.com)
-- is an attendee sitting at NEEDS-ACTION, RSVPs Accept via the
-- `RsvpEvent` harness op, and proves the change landed on THREE planes:
--   1. the local `rsvp_status` column (the action's write-back),
--   2. the MOCK's provider request log (bifrost issued the RSVP PATCH),
--   3. the MOCK's durable state, read back on a follow-up sync as the
--      self attendee's `responseStatus` flipping NEEDS-ACTION -> accepted
--      inside `attendees_json`.
-- Plane 3 is the load-bearing one: a mock that recorded nothing would
-- leave the self attendee at needs-action, so the re-sync assertion
-- fails - exactly the false-green (set-response + local-column only)
-- failure mode section 6.3 exists to catch.

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

-- The self attendee (test@example.com) is the ONLY needs-action
-- attendee in the fixture (Bob is pre-accepted). Its participation
-- status therefore shows up in `attendees_json` as the single
-- `"responseStatus":"needsaction"` key/value pair; once the RSVP
-- Accept has round-tripped through the provider, no attendee is left at
-- needs-action and the self line reads `"responseStatus":"accepted"`.
-- The key/value pair is contiguous in the serialized JSON regardless of
-- object key ordering, so a plain substring search is robust.
local NEEDS_ACTION = '"responseStatus":"needsaction"'
local ACCEPTED = '"responseStatus":"accepted"'

local function has(haystack, needle)
    return haystack ~= nil and string.find(haystack, needle, 1, true) ~= nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_calendar_rsvp_gcal")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-calendar-rsvp-gcal@example.test",
    display_name = "Sync Calendar RSVP Google",
    account_name = "Sync Calendar RSVP Google",
    provider = "gmail_api",
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
local invite = event_by_remote_id(initial.calendar_events, "ev-rsvp-001")
harness.assert(invite ~= nil, "missing invite event")
harness.assert(
    has(invite.attendees_json, NEEDS_ACTION),
    "self attendee should start at needs-action"
)

harness.clear_mock_requests(admin_endpoint)

local rsvped, rsvp_err = client:execute_calendar_plan({
    operations = {
        {
            account_id = account.account_id,
            operation = "RsvpEvent",
            event_id = invite.id,
            response = "accepted",
        },
    },
}, 30)
harness.assert(rsvp_err == nil, "rsvp calendar action failed")
assert_success(rsvped, "rsvp")

-- Plane 1: the local rsvp_status column reflects the action write-back.
local after_rsvp, after_rsvp_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(after_rsvp_err == nil, "post-rsvp TestQueryDbState failed")
local rsvped_row = event_by_remote_id(after_rsvp.calendar_events, "ev-rsvp-001")
harness.assert(rsvped_row ~= nil, "invite event missing after rsvp")
harness.assert_eq(rsvped_row.rsvp_status, "accepted", "local rsvp_status column")

-- Plane 2: bifrost issued the RSVP write to the mock. bifrost's Google
-- RSVP is a read-modify-write: it GETs the event once, patches the self
-- attendee's responseStatus, and PATCHes the whole attendees[] back
-- once. Both hit the calendar-scoped event route. The GET reads as 1
-- (only the mock's `log_request` middleware records it - the read
-- handler adds no log of its own); the PATCH reads as 2 because BOTH
-- the middleware AND the mutating handler log it under the same
-- calendar-scoped command. So the counts below are a measured
-- request-log fact (spec section 6.4), not a double provider write:
-- one GET, one PATCH on the wire.
local requests = harness.mock_requests(admin_endpoint)
harness.assert_eq(
    harness.request_count(
        requests,
        "gcal",
        "GET /calendar/v3/calendars/cal-work/events/ev-rsvp-001"
    ),
    1,
    "expected exactly one Google Calendar RSVP pre-read GET"
)
harness.assert_eq(
    harness.request_count(
        requests,
        "gcal",
        "PATCH /calendar/v3/calendars/cal-work/events/ev-rsvp-001"
    ),
    2,
    "expected the observed two Google Calendar RSVP PATCHes"
)

-- Plane 3 (load-bearing): a follow-up sync re-reads the event from the
-- mock; the self attendee's persisted participation status now reads
-- accepted, and NO attendee is left at needs-action.
local delta, delta_err = client:start_calendar_sync({
    account_id = account.account_id,
}, 30)
harness.assert(delta_err == nil, "post-rsvp delta sync failed")
harness.assert_eq(delta.result, "completed", delta.error or "post-rsvp delta result")

local final, final_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(final_err == nil, "final TestQueryDbState failed")
local final_invite = event_by_remote_id(final.calendar_events, "ev-rsvp-001")
harness.assert(final_invite ~= nil, "invite event missing after delta")
harness.assert(
    has(final_invite.attendees_json, ACCEPTED),
    "mock did not record the accepted participation status"
)
harness.assert(
    not has(final_invite.attendees_json, NEEDS_ACTION),
    "self attendee still at needs-action after RSVP (mock recorded nothing)"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
