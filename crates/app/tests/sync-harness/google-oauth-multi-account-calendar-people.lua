-- description: Google OAuth tokens scope Calendar and People sync to the token-bound account
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: gcal
-- ceiling: 120s

local function mint_token(token_url, account_id, label)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = account_id,
            code = "harness-google-" .. account_id,
            client_id = "ratatoskr-google-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(
        response.access_token ~= nil,
        label .. " /oauth/token did not return access_token"
    )
    return response.access_token
end

local function calendar_by_remote_id(calendars, remote_id)
    for _, calendar in ipairs(calendars) do
        if calendar.remote_id == remote_id then
            return calendar
        end
    end
    return nil
end

local function event_by_subject(events, subject)
    for _, event in ipairs(events) do
        if event.summary == subject then
            return event
        end
    end
    return nil
end

local function contact_by_display_name(contacts, name)
    for _, contact in ipairs(contacts) do
        if contact.display_name == name then
            return contact
        end
    end
    return nil
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)

local primary_token = mint_token(token_url, "account-primary", "primary")
local secondary_token = mint_token(token_url, "account-secondary", "secondary")
harness.assert(primary_token ~= secondary_token, "token store returned duplicate strings")

local dir = harness.data_dir("sync_google_oauth_multi_account_calendar_people")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local future_expiry = 2000000000

local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "Google Primary",
    account_name = "Google Primary",
    provider = "gmail_api",
    access_token = primary_token,
    refresh_token = "primary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-google-harness",
    oauth_token_url = token_url,
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "Google Secondary",
    account_name = "Google Secondary",
    provider = "gmail_api",
    access_token = secondary_token,
    refresh_token = "secondary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-google-harness",
    oauth_token_url = token_url,
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

-- start_sync triggers Gmail mail + People contact sync; calendar sync
-- runs on its own driver, so we kick both per account.
local function run_mail_sync(account_id, label)
    local result, sync_err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " mail sync result"))
end

local function run_calendar_sync(account_id, label)
    local result, sync_err = client:start_calendar_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_calendar_sync failed")
    harness.assert_eq(
        result.result,
        "completed",
        result.error or (label .. " calendar sync result")
    )
end

run_mail_sync(primary.account_id, "primary")
run_mail_sync(secondary.account_id, "secondary")
run_calendar_sync(primary.account_id, "primary")
run_calendar_sync(secondary.account_id, "secondary")

local function query(account_id, label)
    local state, state_err = client:request("TestQueryDbState", {
        account_id = account_id,
        calendar_limit = 10,
        contact_limit = 10,
    })
    harness.assert(state_err == nil, label .. " TestQueryDbState failed")
    return state
end

local primary_state = query(primary.account_id, "primary")
-- Calendar: only primary's calendar lands on primary.
harness.assert_eq(primary_state.calendar_count, 1, "primary calendar count")
harness.assert(
    calendar_by_remote_id(primary_state.calendars, "cal-primary-google") ~= nil,
    "primary missing its own Google calendar"
)
harness.assert(
    calendar_by_remote_id(primary_state.calendars, "cal-secondary-google") == nil,
    "primary leaked secondary's Google calendar"
)
harness.assert_eq(primary_state.calendar_event_count, 1, "primary event count")
harness.assert(
    event_by_subject(primary_state.calendar_events, "Primary planning") ~= nil,
    "primary missing its own Google event"
)
harness.assert(
    event_by_subject(primary_state.calendar_events, "Secondary review") == nil,
    "primary leaked secondary's Google event"
)
-- Contacts: only primary's contact lands on primary.
harness.assert(
    contact_by_display_name(primary_state.contacts, "Primary Contact") ~= nil,
    "primary missing its own contact"
)
harness.assert(
    contact_by_display_name(primary_state.contacts, "Secondary Contact") == nil,
    "primary leaked secondary's contact"
)

local secondary_state = query(secondary.account_id, "secondary")
harness.assert_eq(secondary_state.calendar_count, 1, "secondary calendar count")
harness.assert(
    calendar_by_remote_id(secondary_state.calendars, "cal-secondary-google") ~= nil,
    "secondary missing its own Google calendar"
)
harness.assert(
    calendar_by_remote_id(secondary_state.calendars, "cal-primary-google") == nil,
    "secondary leaked primary's Google calendar"
)
harness.assert_eq(secondary_state.calendar_event_count, 1, "secondary event count")
harness.assert(
    event_by_subject(secondary_state.calendar_events, "Secondary review") ~= nil,
    "secondary missing its own Google event"
)
harness.assert(
    event_by_subject(secondary_state.calendar_events, "Primary planning") == nil,
    "secondary leaked primary's Google event"
)
harness.assert(
    contact_by_display_name(secondary_state.contacts, "Secondary Contact") ~= nil,
    "secondary missing its own contact"
)
harness.assert(
    contact_by_display_name(secondary_state.contacts, "Primary Contact") == nil,
    "secondary leaked primary's contact"
)

local requests = harness.mock_requests(admin_endpoint, { stable = true })
local gcal_list_requests = harness.request_count(
    requests,
    "gcal",
    "GET /calendar/v3/users/me/calendarList"
)
harness.assert(
    gcal_list_requests >= 2,
    "expected at least one Google Calendar list call per account"
)
local people_connections_requests = harness.request_count_prefix(
    requests,
    "people",
    "GET /v1/people/me/connections"
)
harness.assert(
    people_connections_requests >= 2,
    "expected at least one People connections call per account"
)

harness.write_summary({
    correct = 1,
    primary_calendar_count = primary_state.calendar_count,
    secondary_calendar_count = secondary_state.calendar_count,
    primary_contact_count = primary_state.contact_count,
    secondary_contact_count = secondary_state.contact_count,
    gcal_list_requests = gcal_list_requests,
    people_connections_requests = people_connections_requests,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
