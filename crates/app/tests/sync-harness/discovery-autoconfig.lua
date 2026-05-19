-- description: autoconfig XML's OAuth2Unsupported gets upgraded to OAuth2 via OIDC
-- expected: pass
-- fixture: discovery-autoconfig.toml
-- protocol: discovery
-- ceiling: 30s

local discovery_base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(discovery_base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("discovery_autoconfig")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local config, config_err = client:request("TestRunDiscovery", {
    email = "user@corp.test",
})
harness.assert(config_err == nil, config_err or "TestRunDiscovery failed")

-- The cascade should have produced at least one IMAP option from the
-- autoconfig XML. Find it.
harness.assert(config.options ~= nil, "no options array")
local imap_opt = nil
for _, opt in ipairs(config.options) do
    if opt.protocol and opt.protocol.type == "imap" then
        imap_opt = opt
        break
    end
end
harness.assert(imap_opt ~= nil, "no imap protocol option produced from autoconfig")

-- Server fields come from the XML.
harness.assert_eq(
    imap_opt.protocol.incoming.hostname,
    "imap.corp.test",
    "imap hostname"
)
harness.assert_eq(
    imap_opt.protocol.outgoing.hostname,
    "smtp.corp.test",
    "smtp hostname"
)

-- The key assertion: authentication="oauth2" in the XML produces
-- OAuth2Unsupported initially; the OIDC stage running in parallel
-- against corp.test should provide endpoints, and the cascade's
-- post-merge upgrade should turn this into OAuth2 with full endpoints.
harness.assert(imap_opt.auth ~= nil, "no auth on imap option")
harness.assert(imap_opt.auth.method ~= nil, "no auth.method on imap option")
harness.assert_eq(
    imap_opt.auth.method.type,
    "oAuth2",
    "imap auth method should be oAuth2 after upgrade, got " .. tostring(imap_opt.auth.method.type)
)
harness.assert_eq(
    imap_opt.auth.method.authUrl,
    discovery_base .. "/oauth/authorize",
    "upgraded authUrl"
)
harness.assert_eq(
    imap_opt.auth.method.tokenUrl,
    discovery_base .. "/oauth/token",
    "upgraded tokenUrl"
)

-- Diagnostic check: both autoconfig and oidc_discovery stages should
-- report Found.
local stages = {}
for _, diag in ipairs(config.diagnostics or {}) do
    stages[diag.stage] = diag.outcome
end
harness.assert(stages.autoconfig ~= nil, "no autoconfig stage diagnostic")
harness.assert_eq(
    stages.autoconfig.type,
    "found",
    "autoconfig should be found"
)
harness.assert(stages.oidc_discovery ~= nil, "no oidc_discovery stage diagnostic")
harness.assert_eq(
    stages.oidc_discovery.type,
    "found",
    "oidc_discovery should be found"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, shutdown_err or "shutdown failed")
