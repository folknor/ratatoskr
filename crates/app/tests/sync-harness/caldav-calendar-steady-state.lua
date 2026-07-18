-- description: CalDAV Calendar steady-state pull remains bounded and correct
-- expected: pass
-- fixture: graph-calendar-small.toml
-- protocol: caldav
-- ceiling: 120s

local admin = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
local caldav = harness.env("RATATOSKR_TEST_CALDAV_ENDPOINT")
harness.assert(admin ~= nil and caldav ~= nil, "calendar mock endpoint missing")
local dir = harness.data_dir("sync_caldav_calendar_steady_state")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")
harness.assert(client:request("BootReady").ready, "boot.ready returned ready=false")
local account = client:request("TestSeedAccount", { email = "caldav-calendar-steady@example.test", display_name = "CalDAV Calendar Steady", account_name = "CalDAV Calendar Steady", provider = "caldav", caldav_url = caldav, caldav_username = "account-1", caldav_password = "test-password" })
local initial, initial_err = client:start_calendar_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil and initial.result == "completed", initial_err or initial.error or "initial calendar sync failed")
harness.clear_mock_requests(admin)
harness.marker("SYNC_START")
local steady, steady_err = client:start_calendar_sync({ account_id = account.account_id }, 30)
harness.marker("SYNC_END")
harness.assert(steady_err == nil and steady.result == "completed", steady_err or steady.error or "steady calendar sync failed")
local state = client:request("TestQueryDbState", { account_id = account.account_id, calendar_limit = 10 })
harness.assert_eq(state.calendar_count, 2, "steady calendar count")
harness.assert_eq(state.calendar_event_count, 2, "steady event count")
local requests = harness.mock_requests(admin, { stable = true })
harness.assert(harness.request_count(requests, "caldav", "REPORT /calendars/account-1/cal-work/") >= 1, "steady pull missed Work")
harness.write_summary({ correct = 1, calendar_count = state.calendar_count, calendar_event_count = state.calendar_event_count, provider_requests = #requests })
local ok, shutdown_err = client:shutdown()
harness.assert(ok and shutdown_err == nil, "shutdown failed")
