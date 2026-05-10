-- description: Calendar create returns LocalOnly when provider create fails after local insert
-- expected: pass
-- fixture: jmap-calendar-oauth.toml
-- protocol: jmap
-- ceiling: 120s

local function calendar_by_remote_id(calendars, remote_id)
    for _, calendar in ipairs(calendars) do
        if calendar.remote_id == remote_id then
            return calendar
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

local function assert_local_only(completed, label)
    harness.assert(completed ~= nil, label .. " missing completion")
    harness.assert_eq(#completed.results, 1, label .. " result count")
    local result = completed.results[1].result
    harness.assert(result ~= nil, label .. " result missing")
    harness.assert_eq(result.kind, "local_only", label .. " result")
    harness.assert(result.value ~= nil, label .. " value missing")
    harness.assert(
        string.find(result.value.reason or "", "401", 1, true) ~= nil,
        label .. " reason missing 401"
    )
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local token_url = harness.join_url(jmap_endpoint, "oauth/token")
local invalidate_url = harness.join_url(jmap_endpoint, "test/oauth/invalidate")

local dir = harness.data_dir("m6_calendar_create_provider_failure_local_only")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local oauth, oauth_err = client:request("oauth.exchange_code", {
    provider_id = "oidc:saehrimnir",
    token_url = token_url,
    scopes = { "openid", "email", "profile" },
    user_info_url = harness.join_url(jmap_endpoint, "oauth/userinfo"),
    use_pkce = false,
    client_id = "ratatoskr-harness",
    redirect_uri = "http://127.0.0.1/oauth-callback",
    code = "harness-auth-code-calendar-local-only",
})
harness.assert(oauth_err == nil, "oauth.exchange_code failed")
harness.assert(oauth.access_token ~= nil, "oauth ack missing access token")
harness.assert(oauth.refresh_token ~= nil, "oauth ack missing refresh token")
harness.assert(oauth.token_expires_at ~= nil, "oauth ack missing token expiry")

local account, account_err = client:request("TestSeedAccount", {
    email = "m6-calendar-create-provider-failure@example.test",
    display_name = "M6 Calendar Provider Failure",
    account_name = "M6 Calendar Provider Failure",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = oauth.access_token,
    refresh_token = oauth.refresh_token,
    token_expires_at = oauth.token_expires_at,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-harness",
    oauth_token_url = token_url,
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
harness.assert_eq(initial.calendar_count, 2, "initial calendar count")
harness.assert_eq(initial.calendar_event_count, 2, "initial event count")
local work = calendar_by_remote_id(initial.calendars, "cal-work")
harness.assert(work ~= nil, "missing Work calendar")

local invalidated = harness.http_json({
    method = "POST",
    url = invalidate_url,
    body = {
        token = oauth.access_token,
    },
})
harness.assert(invalidated == nil, "token invalidation returned unexpected body")

harness.clear_mock_requests(jmap_endpoint)

local create_input = {
    title = "Local-only provider failure",
    description = "Provider create should fail after local insert",
    location = "Offline Room",
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
assert_local_only(created, "create")

local final, final_err = client:request("TestQueryDbState", {
    account_id = account.account_id,
    calendar_limit = 10,
})
harness.assert(final_err == nil, "final TestQueryDbState failed")
harness.assert_eq(final.calendar_event_count, 3, "local-only create event count")

local local_created = event_by_summary(final.calendar_events, "Local-only provider failure")
harness.assert(local_created ~= nil, "local-only created event missing")
harness.assert_eq(local_created.location, "Offline Room", "local-only location")
harness.assert(
    local_created.remote_event_id == nil,
    "local-only event unexpectedly has remote id"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
