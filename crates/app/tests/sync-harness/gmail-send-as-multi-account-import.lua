-- description: Gmail SendAs signature sync writes each account's aliases under its own bearer token without leakage
-- expected: pass
-- fixture: multi-account-small.toml
-- protocol: gmail
-- ceiling: 120s

local function mint_token(token_url, account_id, label)
    local response = harness.http_json({
        method = "POST",
        url = token_url,
        body = {
            grant_type = "authorization_code",
            account_id = account_id,
            code = "harness-gmail-" .. account_id,
            client_id = "ratatoskr-gmail-harness",
            redirect_uri = "http://127.0.0.1/oauth-callback",
        },
    })
    harness.assert(
        response.access_token ~= nil,
        label .. " /oauth/token did not return access_token"
    )
    return response.access_token
end

local function signature_by_server_id(signatures, server_id)
    for _, sig in ipairs(signatures) do
        if sig.server_id == server_id then
            return sig
        end
    end
    return nil
end

local function imported_signatures(signatures)
    local out = {}
    for _, sig in ipairs(signatures) do
        if sig.server_id ~= nil and sig.server_id ~= "" then
            out[#out + 1] = sig
        end
    end
    return out
end

local SEND_AS_PATCH_PREFIX = "PATCH /gmail/v1/users/me/settings/sendAs"

-- A recorded sendAs PATCH is a "gmail" request-log entry whose command is the
-- write path and whose detail carries the bearer-resolved account, the target
-- address, and the sparse patch body. The saehrimnir mock records the concrete
-- address in detail.sendAsEmail (the command may keep it as a `{sendAsEmail}`
-- template), so match on detail.sendAsEmail rather than parsing the command.
local function find_send_as_patch(requests, address)
    for _, req in ipairs(requests) do
        if req.protocol == "gmail"
            and req.command ~= nil
            and string.find(req.command, SEND_AS_PATCH_PREFIX, 1, true) == 1
            and req.detail ~= nil
            and req.detail.sendAsEmail == address then
            return req
        end
    end
    return nil
end

local function count_send_as_patches(requests)
    local count = 0
    for _, req in ipairs(requests) do
        if req.protocol == "gmail"
            and req.command ~= nil
            and string.find(req.command, SEND_AS_PATCH_PREFIX, 1, true) == 1 then
            count = count + 1
        end
    end
    return count
end

-- Count sendAs PATCHes whose bearer-resolved detail targets one concrete
-- address. Address attribution lives in detail.sendAsEmail (the command may keep
-- the `{sendAsEmail}` template), so key on the detail, mirroring
-- find_send_as_patch. Used to assert per-address push counts without pinning a
-- global total the aux-pass cadence does not guarantee (see the writeback leg).
local function count_send_as_patches_for(requests, address)
    local count = 0
    for _, req in ipairs(requests) do
        if req.protocol == "gmail"
            and req.command ~= nil
            and string.find(req.command, SEND_AS_PATCH_PREFIX, 1, true) == 1
            and req.detail ~= nil
            and req.detail.sendAsEmail == address then
            count = count + 1
        end
    end
    return count
end

local admin_endpoint = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(admin_endpoint ~= nil, "saehrimnir admin endpoint missing")
local token_url = harness.join_url(admin_endpoint, "oauth/token")
harness.clear_mock_requests(admin_endpoint)

-- Mint one bearer per fixture account so saehrimnir's Gmail layer can
-- attribute /gmail/v1/users/me/settings/sendAs to the right account.
local primary_token = mint_token(token_url, "account-primary", "primary")
local secondary_token = mint_token(token_url, "account-secondary", "secondary")
harness.assert(primary_token ~= secondary_token, "token store returned duplicate strings")

local dir = harness.data_dir("sync_gmail_send_as_multi_account")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local future_expiry = 2000000000

local primary, primary_err = client:request("TestSeedAccount", {
    email = "primary@example.com",
    display_name = "Gmail Primary",
    account_name = "Gmail Primary",
    provider = "gmail_api",
    access_token = primary_token,
    refresh_token = "primary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(primary_err == nil, "primary TestSeedAccount failed")

local secondary, secondary_err = client:request("TestSeedAccount", {
    email = "secondary@example.com",
    display_name = "Gmail Secondary",
    account_name = "Gmail Secondary",
    provider = "gmail_api",
    access_token = secondary_token,
    refresh_token = "secondary-refresh-unused",
    token_expires_at = future_expiry,
    oauth_provider = "google",
    oauth_client_id = "ratatoskr-gmail-harness",
    oauth_token_url = token_url,
})
harness.assert(secondary_err == nil, "secondary TestSeedAccount failed")

local function run_sync(account_id, label)
    local result, sync_err = client:start_sync({ account_id = account_id }, 30)
    harness.assert(sync_err == nil, label .. " start_sync failed")
    harness.assert_eq(result.result, "completed", result.error or (label .. " sync result"))
end

run_sync(primary.account_id, "primary")
run_sync(secondary.account_id, "secondary")

-- The normal sync run is finite; attach the resident engine explicitly so the
-- auxiliary Gmail signature reconciliation has its own lifecycle to exercise.
local primary_attach, primary_attach_err = client:request("TestContactPull", {
    account_id = primary.account_id,
})
harness.assert(primary_attach_err == nil, "primary resident attach failed")
harness.assert(primary_attach ~= nil, "primary resident attach returned nil")
local secondary_attach, secondary_attach_err = client:request("TestContactPull", {
    account_id = secondary.account_id,
})
harness.assert(secondary_attach_err == nil, "secondary resident attach failed")
harness.assert(secondary_attach ~= nil, "secondary resident attach returned nil")

local function wait_for_imported_signatures(account_id, expected_count, label)
    local deadline = 15000
    local elapsed = 0
    while elapsed < deadline do
        local state, state_err = client:request("TestQueryDbState", {
            account_id = account_id,
        })
        harness.assert(state_err == nil, label .. " TestQueryDbState failed")
        if #imported_signatures(state.signatures) == expected_count then
            return state
        end
        harness.sleep(250)
        elapsed = elapsed + 250
    end
    harness.assert(false, label .. " timed out waiting for SendAs import")
end

local primary_state = wait_for_imported_signatures(primary.account_id, 2, "primary")
-- TestSeedAccount also inserts a local-only "Harness" signature with
-- server_id = NULL; only Gmail-imported rows carry server_id, so the
-- imported set is what we assert on.
local primary_imported = imported_signatures(primary_state.signatures)
harness.assert_eq(#primary_imported, 2, "primary imported signature count")

local primary_main = signature_by_server_id(primary_state.signatures, "primary@example.com")
harness.assert(primary_main ~= nil, "primary missing main SendAs signature")
harness.assert_eq(
    primary_main.body_html,
    "<p>Primary signature -- send from <b>primary@example.com</b>.</p>",
    "primary main signature HTML"
)
harness.assert(
    string.find(primary_main.name, "Primary Owner", 1, true) ~= nil,
    "primary main signature display name in `name`"
)
harness.assert_eq(
    primary_main.is_default,
    true,
    "primary main signature is_default mirrors Gmail isDefault"
)
harness.assert_eq(
    primary_main.source,
    "gmail_sync",
    "primary main signature source"
)
harness.assert(
    primary_main.server_html_hash ~= nil and primary_main.server_html_hash ~= "",
    "primary main signature server_html_hash populated"
)

local primary_alias = signature_by_server_id(
    primary_state.signatures,
    "primary-alias@example.com"
)
harness.assert(primary_alias ~= nil, "primary missing alias SendAs signature")
harness.assert_eq(
    primary_alias.body_html,
    "<p>Primary alias signature.</p>",
    "primary alias signature HTML"
)
harness.assert_eq(
    primary_alias.is_default,
    false,
    "primary alias signature is_default false"
)

-- Cross-account leakage check: primary must NOT have secondary's identity.
harness.assert(
    signature_by_server_id(primary_state.signatures, "secondary@example.com") == nil,
    "primary leaked secondary's SendAs signature"
)

local secondary_state = wait_for_imported_signatures(secondary.account_id, 1, "secondary")
local secondary_imported = imported_signatures(secondary_state.signatures)
harness.assert_eq(#secondary_imported, 1, "secondary imported signature count")

local secondary_main = signature_by_server_id(
    secondary_state.signatures,
    "secondary@example.com"
)
harness.assert(secondary_main ~= nil, "secondary missing main SendAs signature")
harness.assert_eq(
    secondary_main.body_html,
    "<p>Secondary signature -- different account.</p>",
    "secondary signature HTML"
)
harness.assert_eq(
    secondary_main.is_default,
    true,
    "secondary signature is_default"
)
harness.assert(
    signature_by_server_id(secondary_state.signatures, "primary@example.com") == nil,
    "secondary leaked primary's SendAs signature"
)
harness.assert(
    signature_by_server_id(secondary_state.signatures, "primary-alias@example.com") == nil,
    "secondary leaked primary's alias SendAs signature"
)

-- Writeback (sink) leg. The import above only exercised the PullFromServer arm.
-- Here we mutate the imported PRIMARY alias signature locally so the hash-based
-- state machine sees server-unchanged / local-changed and takes the PushToServer
-- arm, which drives identity_update -> a sendAs PATCH on the mock. We detach the
-- account and re-attach to force a fresh auxiliary pass that performs the push.
local new_alias_html = "<p>Primary alias signature -- locally edited for writeback.</p>"

local _, sig_update_err = client:request("SignatureUpdate", {
    id = primary_alias.id,
    body_html = new_alias_html,
})
harness.assert(sig_update_err == nil, "SignatureUpdate on primary alias failed")

local _, cancel_err = client:request("SyncCancelAccount", {
    account_id = primary.account_id,
})
harness.assert(cancel_err == nil, "SyncCancelAccount for primary failed")

local reattach, reattach_err = client:request("TestContactPull", {
    account_id = primary.account_id,
})
harness.assert(reattach_err == nil, "primary writeback re-attach failed")
harness.assert(reattach ~= nil, "primary writeback re-attach returned nil")

local function wait_for_send_as_patch(address, label)
    local deadline = 30000
    local elapsed = 0
    while elapsed < deadline do
        local reqs = harness.mock_requests(admin_endpoint, { stable = true })
        local patch = find_send_as_patch(reqs, address)
        if patch ~= nil then
            return patch, reqs
        end
        harness.sleep(250)
        elapsed = elapsed + 250
    end
    harness.assert(false, label .. " timed out waiting for sendAs PATCH")
end

local alias_patch, patch_requests =
    wait_for_send_as_patch("primary-alias@example.com", "primary alias")

-- The PATCH targeted the alias address under the PRIMARY account's bearer, and
-- carried exactly the edited signature field (no unintended sparse fields).
harness.assert_eq(
    alias_patch.detail.sendAsEmail,
    "primary-alias@example.com",
    "sendAs PATCH targeted the primary alias address"
)
harness.assert_eq(
    alias_patch.detail.account,
    "account-primary",
    "sendAs PATCH used the primary account's bearer identity"
)
harness.assert(alias_patch.detail.body ~= nil, "sendAs PATCH missing body")
harness.assert_eq(
    alias_patch.detail.body.signature,
    new_alias_html,
    "sendAs PATCH body carried the edited signature HTML"
)
local patched_field_count = 0
for _ in pairs(alias_patch.detail.body) do
    patched_field_count = patched_field_count + 1
end
harness.assert_eq(
    patched_field_count,
    1,
    "sendAs PATCH body carried only the signature field"
)

-- Attribution / no-collateral. The push count is deliberately NOT pinned to
-- exactly one. The reconciliation (gmail_signatures.rs) issues an alias PATCH on
-- every auxiliary pass whose local hash still differs from the stored server
-- hash, and the re-drive here (SyncCancelAccount + TestContactPull) drives the
-- reconciliation more than once: a pass that observes the pre-push alias state
-- re-issues the SAME PATCH (same address, same edited HTML, same primary
-- bearer). That is benign idempotent behavior -- determine_sync_action only
-- pushes an alias whose OWN local hash changed, so the two untouched addresses
-- can never be written no matter how many passes run. What this gate must prove
-- is the identity_update round-trip and its IdentityId attribution, not an exact
-- push count the aux-pass cadence does not guarantee: at least one PATCH hit the
-- alias with the edited HTML under the primary bearer (asserted above), and ZERO
-- PATCHes touched either untouched address.
local alias_patch_count = count_send_as_patches_for(patch_requests, "primary-alias@example.com")
harness.assert(
    alias_patch_count >= 1,
    "at least one sendAs PATCH targeted the primary alias"
)
harness.assert_eq(
    count_send_as_patches_for(patch_requests, "primary@example.com"),
    0,
    "primary main sendAs must not be patched"
)
harness.assert_eq(
    count_send_as_patches_for(patch_requests, "secondary@example.com"),
    0,
    "secondary sendAs must not be patched"
)
local send_as_patch_count = count_send_as_patches(patch_requests)

-- Mock-side persistence of the pushed signature is proven at the saehrimnir
-- level: the mock applies the sendAs PATCH as a sparse merge that preserves
-- read-only fields (isPrimary) and has its own unit test for the recorded write,
-- and the request-log assertions above pin that the PATCH carried exactly the
-- edited HTML under the primary bearer against the alias address. The harness
-- cannot re-read the mock's stored sendAs collection directly: snapshot_state
-- exposes no sendAs collection, and the Gmail sendAs endpoint is bearer-attributed
-- while the harness HTTP helpers cannot attach a per-account bearer. So we confirm
-- the sink leg locally instead, by re-reading the account signature rows. This is
-- a live round-trip check, not a tautology: the reconciliation's conflict arm
-- (determine_sync_action) resolves server-vs-local divergence as ConflictServerWins
-- and would REVERT the local alias body on the next pull if the mock had dropped
-- the pushed HTML; the edit surviving reconciliation is what confirms the server
-- persisted it. The two untouched identities keeping their original HTML confirms
-- no collateral write on the sink leg.
local writeback_state, writeback_state_err = client:request("TestQueryDbState", {
    account_id = primary.account_id,
})
harness.assert(writeback_state_err == nil, "primary writeback TestQueryDbState failed")

local alias_after = signature_by_server_id(
    writeback_state.signatures,
    "primary-alias@example.com"
)
harness.assert(alias_after ~= nil, "primary alias signature missing after writeback")
harness.assert_eq(
    alias_after.body_html,
    new_alias_html,
    "primary alias retained the edited signature after push/reconcile (not reverted by a conflicting pull)"
)
harness.assert(
    alias_after.server_html_hash ~= nil and alias_after.server_html_hash ~= "",
    "primary alias server_html_hash populated after push"
)

local main_after = signature_by_server_id(writeback_state.signatures, "primary@example.com")
harness.assert(main_after ~= nil, "primary main signature missing after writeback")
harness.assert_eq(
    main_after.body_html,
    "<p>Primary signature -- send from <b>primary@example.com</b>.</p>",
    "primary main signature left untouched by the alias writeback"
)

-- Secondary account: untouched by the primary alias writeback, and still no leakage.
local secondary_after_state, secondary_after_err = client:request("TestQueryDbState", {
    account_id = secondary.account_id,
})
harness.assert(secondary_after_err == nil, "secondary post-writeback TestQueryDbState failed")
local secondary_after = signature_by_server_id(
    secondary_after_state.signatures,
    "secondary@example.com"
)
harness.assert(secondary_after ~= nil, "secondary main signature missing after writeback")
harness.assert_eq(
    secondary_after.body_html,
    "<p>Secondary signature -- different account.</p>",
    "secondary signature untouched by the primary alias writeback"
)
harness.assert(
    signature_by_server_id(secondary_after_state.signatures, "primary-alias@example.com") == nil,
    "secondary leaked primary's alias after writeback"
)

-- Aggregate presence check only: at least one bifrost sendAs LIST call reached
-- the mock. It does NOT establish per-account attribution -- the signature-table
-- assertions and the cross-account no-leakage / writeback-PATCH checks above are
-- what prove each account's aliases are sourced and sunk under its own bearer.
local requests = harness.mock_requests(admin_endpoint, { stable = true })
local send_as_list_requests = harness.request_count(
    requests,
    "gmail",
    "GET /gmail/v1/users/me/settings/sendAs"
)
harness.assert(
    send_as_list_requests >= 1,
    "expected at least one bifrost sendAs list call"
)

harness.write_summary({
    correct = 1,
    primary_imported_count = #primary_imported,
    secondary_imported_count = #secondary_imported,
    gmail_send_as_list_requests = send_as_list_requests,
    gmail_send_as_patch_requests = send_as_patch_count,
})

local ok, shutdown_err = client:shutdown()
harness.assert(ok, "shutdown failed")
harness.assert(shutdown_err == nil, "shutdown returned error")
