-- description: Gmail user-label create/rename/recolor/delete round-trips through engine.container_*
-- expected: pass
-- fixture: jmap-small.toml
-- protocol: gmail
-- ceiling: 120s
--
-- B6b container-CRUD gate (Gmail). Gmail containers are LABELS, so the
-- label CRUD action handlers (actions::label::{create,rename,recolor,delete})
-- dispatch ContainerKind::Label through engine.container_create /
-- container_rename (with the style arg for recolor) / container_delete. A
-- follow-up resync's containers_list must reflect each step on the server.

local function label_by_name(labels, name)
    for _, label in ipairs(labels) do
        if label.name == name then
            return label
        end
    end
    return nil
end

local function resync(client, account_id, label)
    local r, e = client:start_sync({ account_id = account_id }, 30)
    harness.assert(e == nil, label .. " resync failed")
    harness.assert_eq(r.result, "completed", r.error or (label .. " resync result"))
end

local function db_state(client, account_id)
    local s, e = client:request("TestQueryDbState", { account_id = account_id })
    harness.assert(e == nil, "TestQueryDbState failed")
    return s
end

local function crud(client, account_id, params)
    params.account_id = account_id
    local ack, e = client:request("TestContainerCrud", params)
    if e ~= nil then
        harness.assert(false, "TestContainerCrud " .. params.op .. " failed: " .. tostring(e.detail))
    end
    harness.assert(ack.ok, params.op .. " outcome not Success: " .. tostring(ack.outcome))
    return ack
end

-- Gmail is OAuth-only: mint a bearer for the fixture's primary account
-- (account-1) off the mock OAuth provider before seeding.
local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
local token_response = harness.http_json({
    method = "POST",
    url = token_url,
    body = {
        grant_type = "authorization_code",
        account_id = "account-1",
        code = "harness-gmail-crud-account-1",
        client_id = "ratatoskr-gmail-harness",
        redirect_uri = "http://127.0.0.1/oauth-callback",
    },
})
harness.assert(token_response.access_token ~= nil, "/oauth/token did not return access_token")

local dir = harness.data_dir("sync_gmail_container_crud")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local account, account_err = client:request("TestSeedAccount", {
    email = "sync-gmail-crud@example.test",
    display_name = "Sync Gmail CRUD",
    account_name = "Sync Gmail CRUD",
    provider = "gmail_api",
    access_token = token_response.access_token,
    refresh_token = "gmail-crud-refresh-unused",
    token_expires_at = 2000000000,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(account_err == nil, "TestSeedAccount failed")

local initial, initial_err = client:start_sync({ account_id = account.account_id }, 30)
harness.assert(initial_err == nil, "initial start_sync failed")
harness.assert_eq(initial.result, "completed", initial.error or "initial sync result")

local label = crud(client, account.account_id, { op = "label_create", name = "HarnessTag" })
resync(client, account.account_id, "post-create")
local state = db_state(client, account.account_id)
harness.assert(label_by_name(state.labels, "HarnessTag") ~= nil, "label missing after create")

crud(client, account.account_id, { op = "label_rename", id = label.new_id, name = "HarnessTagRenamed" })
resync(client, account.account_id, "post-rename")
state = db_state(client, account.account_id)
harness.assert(label_by_name(state.labels, "HarnessTagRenamed") ~= nil, "renamed label missing on the server")
harness.assert(label_by_name(state.labels, "HarnessTag") == nil, "old label name still present after rename")

crud(client, account.account_id, {
    op = "label_recolor",
    id = label.new_id,
    name = "HarnessTagRenamed",
    color_bg = "#fb4c2f",
    color_fg = "#ffffff",
})
resync(client, account.account_id, "post-recolor")
state = db_state(client, account.account_id)
local recolored = label_by_name(state.labels, "HarnessTagRenamed")
harness.assert(recolored ~= nil, "label missing after recolor")
harness.assert_eq(recolored.server_color_bg, "#fb4c2f", "label recolor did not round-trip on the server")

crud(client, account.account_id, { op = "label_delete", id = label.new_id })
resync(client, account.account_id, "post-delete")
state = db_state(client, account.account_id)
harness.assert(label_by_name(state.labels, "HarnessTagRenamed") == nil, "label still present after delete")

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
