-- description: WebFinger resolves chained OIDC issuer; cascade returns endpoints
-- expected: pass
-- fixture: discovery-webfinger.toml
-- protocol: discovery
-- ceiling: 30s

-- Saehrimnir mounts discovery on its JMAP listener; ratatoskr reuses the
-- JMAP endpoint env var as the discovery base.
local discovery_base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(discovery_base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("discovery_webfinger")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Run the discovery cascade. Saehrimnir's fixture wires WebFinger at the
-- corp.test prefix delegating to /idp/realms/corp, and the chained OIDC
-- discovery doc at that prefix. Ratatoskr's WebFinger probe should fire
-- first, follow the href, then probe_issuer should pick up the document.
local config, config_err = client:request("TestRunDiscovery", {
    email = "user@corp.test",
})
harness.assert(config_err == nil, config_err or "TestRunDiscovery failed")

-- DiscoveredConfig serializes with serde rename_all="camelCase".
harness.assert(config.oidcEndpoints ~= nil, "no oidcEndpoints in discovery result")
local oidc = config.oidcEndpoints
harness.assert_eq(
    oidc.issuerUrl,
    discovery_base .. "/idp/realms/corp",
    "issuerUrl"
)
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

-- Both discovery stages should report Found. StageOutcome serializes as
-- a tagged enum: { "type": "found", "count": N } when matched.
local stages = {}
for _, diag in ipairs(config.diagnostics or {}) do
    stages[diag.stage] = diag.outcome
end
harness.assert(stages.webfinger ~= nil, "no webfinger stage diagnostic")
harness.assert(stages.oidc_discovery ~= nil, "no oidc_discovery stage diagnostic")
harness.assert_eq(
    stages.webfinger.type,
    "found",
    "webfinger stage didn't resolve"
)
harness.assert_eq(
    stages.oidc_discovery.type,
    "found",
    "oidc_discovery stage didn't resolve"
)

local ok, shutdown_err = client:shutdown()
harness.assert(ok, shutdown_err or "shutdown failed")
