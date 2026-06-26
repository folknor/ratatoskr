-- description: JMAP retry-budget exhaustion pauses the resident account and resume unblocks it
-- expected: pass
-- fixture: jmap-changes-budget-exhaustion.lua
-- protocol: jmap
-- ceiling: 300s

local function wait_for_account_paused(queue, account_id, timeout_s)
    local deadline = harness.now_ms() + timeout_s * 1000
    while harness.now_ms() < deadline do
        local notification = queue:recv(1)
        if notification ~= nil
            and notification.method == "sync.account_paused"
            and notification.account_id == account_id
        then
            return notification
        end
    end
    return nil
end

local function assert_failed(result, err, label)
    harness.assert(err == nil, label .. " transport failed")
    harness.assert(result ~= nil, label .. " returned nil result")
    harness.assert_eq(result.result, "failed", label .. " result")
    harness.assert(result.error ~= nil, label .. " missing error")
end

local jmap_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(jmap_endpoint ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")
local token_url = harness.join_url(jmap_endpoint, "oauth/token")
local fail_open_url = harness.join_url(jmap_endpoint, "test/jmap/fail-open")

local token = harness.http_json({
    method = "POST",
    url = token_url,
    body = {
        grant_type = "authorization_code",
        account_id = "account-1",
        code = "harness-jmap-pause-resume",
        client_id = "ratatoskr-jmap-pause-resume",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(token.access_token ~= nil, "/oauth/token did not return access_token")

local dir = harness.data_dir("sync_jmap_pause_resume")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local queue = client:notifications()

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-jmap-pause-resume@example.test",
    display_name = "Sync JMAP Pause Resume",
    account_name = "Sync JMAP Pause Resume",
    provider = "jmap",
    auth_method = "oauth2",
    access_token = token.access_token,
    refresh_token = "jmap-pause-resume-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "oidc:saehrimnir",
    oauth_client_id = "ratatoskr-jmap-pause-resume",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")
local account_id = account.account_id

local initial, initial_err = client:start_sync({
    account_id = account_id,
}, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

-- The resident auxiliary JMAP pass runs once shortly after attach and also
-- opens the JMAP session. Let that bounded pass clear before arming the
-- open-failure budget, so the forced opens are consumed by bifrost's
-- account-reopen budget rather than by auxiliary contact/signature work.
harness.sleep(6)

-- Load-bearing EXHAUSTION, distinct from the fixture's accountNotFound trigger.
-- The fixture's Email/changes accountNotFound (call 3) makes the engine enter
-- its account-reopen path; this arming forces every reopen (factory.open) to
-- fail so the 3-attempt budget exhausts and the engine emits Terminated +
-- Pause(RetryBudgetExhausted). Neither alone produces the pause: without this,
-- the triggered reopen would succeed and the account would recover.
-- db34ab4's JMAP factory does bounded session-fetch retry inside each
-- factory.open. Twelve forced session failures cover the engine's three
-- account-reopen attempts plus one spare open for unrelated resident work.
local armed = harness.http_json({
    method = "POST",
    url = fail_open_url,
    body = {
        count = 12,
    },
})
harness.assert_eq(armed.remaining, 12, "forced session-open failure budget")

local paused_result, paused_err = client:start_sync({
    account_id = account_id,
}, 30)
assert_failed(paused_result, paused_err, "pause-driving start_sync")

local paused = wait_for_account_paused(queue, account_id, 30)
harness.assert(
    paused ~= nil,
    "sync.account_paused not observed after forced JMAP session-open failures"
)
harness.assert(
    paused.reason == "needs_attention",
    "unexpected pause reason: " .. tostring(paused.reason)
)

local parked_started = harness.now_ms()
local parked, parked_err = client:start_sync({
    account_id = account_id,
}, 5)
local parked_elapsed = harness.now_ms() - parked_started
assert_failed(parked, parked_err, "parked start_sync")
harness.assert(
    parked_elapsed < 5000,
    "parked start_sync did not fail fast: " .. tostring(parked_elapsed) .. "ms"
)

local cleared = harness.http({
    method = "DELETE",
    url = fail_open_url,
})
harness.assert_eq(cleared.status, 204, "clear forced session-open failure status")

local resume, resume_err = client:request("sync.resume_account", {
    account_id = account_id,
})
harness.assert(resume_err == nil, "sync.resume_account failed")
harness.assert(resume.resumed, "sync.resume_account did not resume account")

local recovered, recovered_err = client:start_sync({
    account_id = account_id,
}, 60)
harness.assert(recovered_err == nil, "recovered start_sync transport failed")
harness.assert_eq(recovered.result, "completed", recovered.error or "recovered sync result")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
