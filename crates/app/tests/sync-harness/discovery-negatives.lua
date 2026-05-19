-- description: cascade rejects malformed JRD, non-HTTPS href, issuer self-claim mismatch
-- expected: pass
-- fixture: discovery-negatives.toml
-- protocol: discovery
-- ceiling: 30s

local discovery_base = harness.env("RATATOSKR_TEST_JMAP_ENDPOINT")
harness.assert(discovery_base ~= nil, "RATATOSKR_TEST_JMAP_ENDPOINT missing")

local dir = harness.data_dir("discovery_negatives")
local client, err = harness.spawn(dir)
harness.assert(err == nil, "spawn failed")

local ready, ready_err = client:request("BootReady")
harness.assert(ready_err == nil, "boot.ready failed")
harness.assert(ready.ready, "boot.ready returned ready=false")

-- Helper: run discovery for an email, return the config.
local function discover(email)
    local config, derr = client:request("TestRunDiscovery", { email = email })
    harness.assert(derr == nil, derr or ("TestRunDiscovery failed for " .. email))
    return config
end

-- Positive control: harness wire works; cascade resolves OIDC endpoints.
local ok = discover("user@ok.test")
harness.assert(
    ok.oidcEndpoints ~= nil,
    "ok.test: should have resolved OIDC endpoints"
)
harness.assert_eq(
    ok.oidcEndpoints.issuerUrl,
    "https://ok.test",
    "ok.test issuerUrl"
)

-- Malformed JRD: WebFinger parser fails, no chained probe, bare-domain
-- OIDC also 404s -> oidcEndpoints absent.
local malformed = discover("user@malformed.test")
harness.assert(
    malformed.oidcEndpoints == nil,
    "malformed.test: malformed JRD should not yield oidcEndpoints"
)

-- Non-HTTPS href: is_valid_https_url rejects the href before fetching.
-- The chained probe never fires.
local insecure = discover("user@insecure.test")
harness.assert(
    insecure.oidcEndpoints == nil,
    "insecure.test: http:// href should be rejected by is_valid_https_url"
)

-- Issuer self-claim mismatch: WebFinger -> /wrong-issuer; the OIDC doc
-- there advertises issuer=https://attacker.example.com which doesn't
-- match the URL we fetched it from. probe_issuer rejects.
local mismatched = discover("user@mismatched.test")
harness.assert(
    mismatched.oidcEndpoints == nil,
    "mismatched.test: issuer self-claim mismatch should be rejected by probe_issuer"
)

local ok2, shutdown_err = client:shutdown()
harness.assert(ok2, shutdown_err or "shutdown failed")
