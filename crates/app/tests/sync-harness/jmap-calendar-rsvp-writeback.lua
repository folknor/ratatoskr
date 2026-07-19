-- description: JMAP calendar RSVP (Accept) writeback round-trip
-- expected: pass
-- fixture: calendar-rsvp-jmap.toml
-- protocol: jmap
-- ceiling: 120s

-- B7b section-6.3 RSVP-writeback gate. See gcal-calendar-rsvp-writeback
-- for the three-plane proof structure. JMAP RSVP is a read-modify-write
-- PatchObject: bifrost CalendarEvent/get resolves the self participant
-- (matched by the account's self email = test@example.com), then
-- CalendarEvent/set sends `participants/{id}/participationStatus`.

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

local NEEDS_ACTION = '"responseStatus":"needsaction"'
local ACCEPTED = '"responseStatus":"accepted"'

local function has(haystack, needle)
    return haystack ~= nil and string.find(haystack, needle, 1, true) ~= nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
harness.clear_mock_requests(admin_endpoint)

local dir = harness.data_dir("sync_calendar_rsvp_jmap")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-calendar-rsvp-jmap@example.test",
    display_name = "Sync Calendar RSVP JMAP",
    account_name = "Sync Calendar RSVP JMAP",
    provider = "jmap",
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

-- Plane 2: bifrost issued the get + set the JMAP RSVP requires.
local requests = harness.mock_requests(admin_endpoint)
harness.assert(
    harness.request_count(requests, "jmap", "CalendarEvent/get") >= 1,
    "expected a JMAP CalendarEvent/get pre-read for the RSVP"
)
harness.assert_eq(
    harness.request_count(requests, "jmap", "CalendarEvent/set"),
    1,
    "expected exactly one JMAP CalendarEvent/set for the RSVP"
)

-- Plane 3 (load-bearing): a follow-up sync re-reads the event; the self
-- attendee's persisted participation status now reads accepted, and no
-- attendee is left at needs-action.
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
