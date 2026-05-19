-- description: cascade falls back to bare-domain OIDC probe when WebFinger 404s
-- expected: pass
-- fixture: discovery-oidc-bare-domain.toml
-- protocol: discovery
-- ceiling: 30s

local discovery_base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(discovery_base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("discovery_oidc_bare_domain")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

local config, config_err = client:request("TestRunDiscovery", {
    email = "user@corp.test",
})
harness.assert(config_err == nil, config_err or "TestRunDiscovery failed")

-- Bare-domain probe should land the endpoints. Issuer matches the email
-- domain's prefix exactly (no WebFinger delegation involved).
harness.assert(config.oidcEndpoints ~= nil, "no oidcEndpoints in result")
local oidc = config.oidcEndpoints
-- Bare-domain probe: issuer URL stays in pre-rewrite space (what
-- ratatoskr believes the IdP is at). Endpoint URLs below get
-- emit-time-prefixed to saehrimnir because they were path-relative
-- in the fixture.
harness.assert_eq(oidc.issuerUrl, "https://corp.test", "issuerUrl")
harness.assert_eq(
    oidc.authUrl,
    discovery_base .. "/oauth/authorize",
    "authUrl"
)
harness.assert_eq(
    oidc.tokenUrl,
    discovery_base .. "/oauth/token",
    "tokenUrl"
)

-- Diagnostics: webfinger should be NotFound, oidc_discovery should be Found.
local stages = {}
for _, diag in ipairs(config.diagnostics or {}) do
    stages[diag.stage] = diag.outcome
end
harness.assert(stages.webfinger ~= nil, "no webfinger stage diagnostic")
harness.assert(stages.oidc_discovery ~= nil, "no oidc_discovery stage diagnostic")
harness.assert_eq(
    stages.webfinger.type,
    "notFound",
    "webfinger should be notFound (no webfinger table in fixture)"
)
harness.assert_eq(
    stages.oidc_discovery.type,
    "found",
    "oidc_discovery should be found (bare-domain probe)"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, shutdown_err or "shutdown failed")
