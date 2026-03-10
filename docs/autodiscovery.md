# Email Autodiscovery: Specification & Implementation Plan

**Date**: March 2026
**Status**: Planning
**Goal**: Replace the hardcoded provider list in `autoDiscovery.ts` with a Rust-native multi-protocol discovery system that can configure any email account — IMAP, JMAP, Gmail API, or Microsoft Graph — from just an email address.

---

## Table of Contents

- [Problem Statement](#problem-statement)
- [Current State](#current-state)
- [Target State](#target-state)
- [Discovery Cascade](#discovery-cascade)
- [Protocol Detection](#protocol-detection)
- [Data Model](#data-model)
- [Stage 1: Hardcoded Provider Registry](#stage-1-hardcoded-provider-registry)
- [Stage 2: Autoconfig XML Fetch](#stage-2-autoconfig-xml-fetch)
- [Stage 3: MX-Based Domain Resolution](#stage-3-mx-based-domain-resolution)
- [Stage 4: JMAP Well-Known Discovery](#stage-4-jmap-well-known-discovery)
- [Stage 5: Port Probing](#stage-5-port-probing)
- [OAuth Integration](#oauth-integration)
- [Username Format Detection](#username-format-detection)
- [Validation & Verification](#validation--verification)
- [Tauri Commands](#tauri-commands)
- [TS Integration](#ts-integration)
- [File Structure](#file-structure)
- [What We Skip](#what-we-skip)
- [Reference: Thunderbird's Approach](#reference-thunderbirds-approach)
- [Open Questions](#open-questions)

---

## Problem Statement

Today a user adding a non-Gmail account hits one of two paths:

1. **Known provider** (10 domains) → hardcoded IMAP/SMTP settings, works instantly.
2. **Unknown provider** → naively guesses `imap.{domain}` / `smtp.{domain}` with SSL. This fails for any provider that doesn't follow that naming convention, or that requires OAuth, or that supports JMAP instead of (or alongside) IMAP.

This is inadequate for a multi-protocol email client. We need discovery that:

- Works for the long tail of email providers, not just 10 hardcoded domains.
- Detects **which protocols** a provider supports (IMAP, JMAP, Graph) and prefers the best one.
- Detects **auth method** (password, OAuth2) and routes the UI accordingly.
- Handles **custom domains on hosted providers** (e.g., `user@company.com` with MX pointing to `outlook.com`).
- Runs entirely in Rust — no IPC round-trips for DNS lookups or port probes.

---

## Current State

### TS-side (`src/services/imap/autoDiscovery.ts`)

- 10 hardcoded `WellKnownProvider` entries with IMAP/SMTP host/port/security.
- `discoverSettings(email)` → match domain → return settings or guess `imap.{domain}`.
- OAuth provider IDs for Microsoft and Yahoo only.
- No network requests. No MX lookups. No autoconfig. No JMAP detection.

### Rust-side (`src-tauri/src/jmap/auto_discovery.rs`)

- 2 hardcoded JMAP providers (Fastmail, messagingengine.com).
- `.well-known/jmap` probe via bare `reqwest::get`.
- No integration with the IMAP discovery path.

### OAuth (`src/services/oauth/providers.ts`)

- Microsoft and Yahoo OAuth configs with auth/token URLs, scopes, PKCE flags.
- Separate from discovery — manually wired via `oauthProviderId` on known providers.

These three systems are disconnected. Discovery needs to unify them.

---

## Target State

A single Rust module (`src-tauri/src/discovery/`) that:

1. Takes an email address.
2. Runs a prioritized cascade of discovery methods.
3. Returns a `DiscoveredConfig` with one or more protocol options, ranked by preference.
4. Includes auth method, OAuth provider ID (if applicable), and username format.
5. Is exposed as a single Tauri command (`discover_email_config`).
6. The TS side calls this one command and uses the result to populate the account setup UI.

The existing `autoDiscovery.ts` and `jmap/auto_discovery.rs` are both replaced.

---

## Discovery Cascade

All stages run **concurrently** and their results are **collected and merged** into a single ranked options list. This is not a first-success-wins model — the goal is to gather protocol options from every stage that has something to contribute, then deduplicate and rank. The only exception is port probing (Stage 5), which is gated to avoid unnecessary socket connections when higher-confidence stages produce results.

```
┌──────────────────────────────────────────────────────────┐
│  Input: email address (e.g., user@example.com)           │
│  Extract domain: example.com                             │
└──────────────┬───────────────────────────────────────────┘
               │
               │  All stages run concurrently:
               │
    ┌──────────▼──────────┐
    │  Stage 1: Registry  │  ◄── Hardcoded known providers (instant)
    │                     │      Covers Gmail, Outlook, Yahoo, iCloud,
    │                     │      Fastmail, Zoho, AOL, GMX, etc.
    │                     │      May return multiple options (e.g., Graph + IMAP)
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │  Stage 2: Autoconfig│  ◄── Fetch XML from well-known URLs (network)
    │  XML fetch          │      https://autoconfig.{domain}/mail/config-v1.1.xml
    └──────────┬──────────┘      https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml
               │
    ┌──────────▼──────────┐
    │  Stage 3: MX lookup │  ◄── DNS MX → extract base domain →
    │  + re-check 1 & 2   │      re-run registry + autoconfig on MX domain
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │  Stage 4: JMAP      │  ◄── https://{domain}/.well-known/jmap
    │  .well-known probe  │
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │  Stage 5: Port probe│  ◄── Gated: only runs if stages 1-4 produce no
    │                     │      IMAP option after 3s. Cancelled if they do.
    └──────────┬──────────┘
               │
    ┌──────────▼──────────┐
    │  Collect, dedup,    │  ◄── Merge all stage results, remove duplicates
    │  rank               │      (same protocol+host), rank by protocol preference
    └─────────────────────┘
```

### Why collect-all, not first-success

Different stages discover different protocols. The registry might know that Fastmail supports IMAP, while Stage 4 discovers that it also supports JMAP. MX lookup might reveal that `user@company.com` is hosted on Outlook, while the registry has Graph + IMAP details for Outlook. Cancelling stages on first success would lose these complementary results and break the multi-protocol design.

### Concurrency model

```rust
// Pseudocode — all stages run to completion (or timeout), results collected
let (registry_results, autoconfig_results, mx_results, jmap_results, probe_results) =
    tokio::join!(
        stage_registry(domain),
        stage_autoconfig(domain, &email),
        stage_mx_lookup(domain),
        stage_jmap_wellknown(domain),
        stage_port_probe(domain, &cancel_token),  // gated, see below
    );

let merged = merge_and_rank(
    registry_results,
    autoconfig_results,
    mx_results,
    jmap_results,
    probe_results,
);
```

### Port probing gate

Stage 5 (port probing) is the exception to the collect-all model. It waits ~3 seconds before starting, then checks whether any other stage has already produced an IMAP/SMTP result. If so, port probing is cancelled — it would only rediscover the same servers at lower confidence. If no IMAP option exists after the delay, port probing runs as a last resort.

### Timeouts

| Stage | Timeout | Rationale |
|-------|---------|-----------|
| Registry | 0ms | Pure lookup, no I/O |
| Autoconfig XML | 5s per URL | HTTP fetch; 2-4 URLs tried |
| MX lookup | 5s | DNS query + follow-up HTTP |
| JMAP well-known | 5s | Single HTTP request |
| Port probing | 3s per probe | TCP connect + protocol banner |
| **Overall** | **15s** | User-facing operation, must feel fast |

---

## Protocol Detection

Discovery doesn't just find IMAP settings — it determines **which protocol to use**. The result includes all detected protocols, ranked by preference:

| Priority | Protocol | When detected |
|----------|----------|---------------|
| 1 | **Gmail API** | Registry match for `gmail.com` / `googlemail.com` domains |
| 2 | **Microsoft Graph** | Registry match for `outlook.com` / `hotmail.com` / `live.com` + custom domains via MX |
| 3 | **JMAP** | `.well-known/jmap` returns 200, or autoconfig XML advertises JMAP |
| 4 | **IMAP/SMTP** | Autoconfig XML, MX-based lookup, or port probing |

The UI uses the highest-priority protocol by default but can show alternatives. For example, an Outlook.com user gets Graph as the primary option and IMAP as a fallback.

### Why this ranking

- **Gmail API** and **Graph** are richer than IMAP for their respective ecosystems (native labels, delta sync, send-as, categories). They're preferred when available.
- **JMAP** is preferred over IMAP because it's stateless HTTP (no connection management), has native threading, cleaner delta sync, and batched operations.
- **IMAP** is the universal fallback — virtually every email provider supports it.

### Multi-protocol results

A single discovery run can return multiple options:

```
user@fastmail.com →
  [0] JMAP  (via .well-known/jmap → api.fastmail.com)
  [1] IMAP  (via registry → imap.fastmail.com:993)

user@outlook.com →
  [0] Graph (via registry → Microsoft Graph API)
  [1] IMAP  (via registry → imap-mail.outlook.com:993, OAuth2)

user@selfhosted.org →
  [0] JMAP  (via .well-known/jmap)    ← if server supports it
  [1] IMAP  (via autoconfig XML)      ← fallback
```

---

## Data Model

### `DiscoveredConfig` — the top-level result

```rust
/// Complete discovery result for an email address.
pub struct DiscoveredConfig {
    /// The email address that was queried.
    pub email: String,
    /// The domain extracted from the email.
    pub domain: String,
    /// Protocol options, ranked by preference (index 0 = best).
    /// Each option carries its own `source` for provenance.
    pub options: Vec<ProtocolOption>,
    /// If MX lookup resolved the domain to a different provider,
    /// the base domain of the MX host (e.g., "outlook.com").
    pub resolved_domain: Option<String>,
    /// Per-stage diagnostics: which stages ran, what they found or why they failed.
    /// For debugging/advanced UI, not for routing logic.
    pub diagnostics: Vec<StageDiagnostic>,
}

/// What happened in a single discovery stage.
pub struct StageDiagnostic {
    pub stage: &'static str,
    pub duration_ms: u64,
    pub outcome: StageOutcome,
}

pub enum StageOutcome {
    /// Stage produced one or more protocol options.
    Found { count: usize },
    /// Stage ran but found nothing (not an error).
    NotFound,
    /// Stage failed (network error, parse error, etc.).
    Error { message: String },
    /// Stage was skipped or cancelled (e.g., port probing gated).
    Skipped,
}
```

There is intentionally no top-level `source` field. Each `ProtocolOption` carries its own `source` because different options come from different stages (e.g., Graph from MX lookup, JMAP from .well-known, IMAP from autoconfig). A single top-level source would be ambiguous in this merged model.

### `ProtocolOption` — one way to connect

```rust
/// A single protocol option discovered for the email address.
pub struct ProtocolOption {
    pub protocol: Protocol,
    pub auth: AuthConfig,
    /// Display name for the provider (e.g., "Fastmail", "Outlook.com").
    pub provider_name: Option<String>,
    /// Where this option came from.
    pub source: DiscoverySource,
}

pub enum Protocol {
    GmailApi,
    MicrosoftGraph,
    Jmap {
        session_url: String,
    },
    Imap {
        incoming: ServerConfig,
        outgoing: ServerConfig,
    },
}

pub struct ServerConfig {
    pub hostname: String,
    pub port: u16,
    pub security: Security,
    /// Username pattern. See "Username Format Detection" section.
    pub username: UsernameFormat,
}

pub enum Security {
    Tls,       // Implicit TLS (connect encrypted)
    StartTls,  // Upgrade after plaintext connect
    None,      // No encryption (should warn user)
}

pub enum UsernameFormat {
    /// Use the full email address as username.
    EmailAddress,
    /// Use the local part only (before @).
    LocalPart,
    /// Use a specific fixed username (rare; e.g., some Exchange setups).
    Custom(String),
}
```

### `AuthConfig` — how to authenticate

```rust
pub struct AuthConfig {
    /// Preferred auth method.
    pub method: AuthMethod,
    /// Alternative auth methods, in fallback order.
    pub alternatives: Vec<AuthMethod>,
}

pub enum AuthMethod {
    /// Username + password (PLAIN, LOGIN, or CRAM-MD5).
    Password,
    /// OAuth2 with a known provider. Includes everything the
    /// OAuth flow needs — no separate lookup required.
    OAuth2 {
        provider_id: String,
        auth_url: String,
        token_url: String,
        scopes: Vec<String>,
        use_pkce: bool,
    },
    /// The provider requires OAuth2 (per autoconfig XML) but we don't
    /// have endpoint metadata for it. This option cannot be used until
    /// support is added. See "When autoconfig says OAuth2 but we lack
    /// endpoints" in the OAuth Integration section.
    OAuth2Unsupported {
        provider_domain: String,
    },
}
```

### `DiscoverySource` — provenance tracking

```rust
/// Where a discovery result came from. Used for diagnostics and
/// to help decide trust level (registry is trusted, port probe is guessed).
pub enum DiscoverySource {
    /// Hardcoded in our provider registry.
    Registry,
    /// Mozilla-format autoconfig XML from the provider's server.
    AutoconfigXml { url: String },
    /// DNS MX record resolved to a known provider.
    MxLookup { mx_domain: String },
    /// JMAP .well-known discovery (RFC 8620 §2.2).
    JmapWellKnown,
    /// Port probing (least reliable).
    PortProbe,
}
```

---

## Stage 1: Hardcoded Provider Registry

**Purpose**: Instant results for well-known providers. No network. Highest trust.

This replaces both the TS `wellKnownProviders` array and the Rust `KNOWN_PROVIDERS` slice. It's the single source of truth for all provider-specific knowledge.

### What the registry contains

Each entry maps one or more domains to a complete configuration:

```rust
struct RegistryEntry {
    /// Domains this entry matches (e.g., ["outlook.com", "hotmail.com", "live.com"]).
    domains: &'static [&'static str],
    /// Provider display name.
    name: &'static str,
    /// Protocol options, in preference order.
    options: &'static [RegistryProtocol],
}

enum RegistryProtocol {
    GmailApi,
    MicrosoftGraph,
    Jmap {
        session_url: &'static str,
    },
    Imap {
        imap_host: &'static str,
        imap_port: u16,
        imap_security: Security,
        smtp_host: &'static str,
        smtp_port: u16,
        smtp_security: Security,
        accept_invalid_certs: bool,
    },
}
```

### Entries to include

Migrated from `autoDiscovery.ts` + `jmap/auto_discovery.rs` + `oauth/providers.ts`, unified:

| Provider | Domains | Primary protocol | Fallback | Auth | OAuth provider |
|----------|---------|-----------------|----------|------|---------------|
| **Gmail** | gmail.com, googlemail.com | Gmail API | IMAP (imap.gmail.com:993) | OAuth2 | google |
| **Outlook/Hotmail** | outlook.com, hotmail.com, live.com, msn.com, outlook.co.uk, hotmail.co.uk | Graph | IMAP (imap-mail.outlook.com:993) | OAuth2 | microsoft |
| **Yahoo** | yahoo.com, yahoo.co.uk, yahoo.co.jp, ymail.com | IMAP | — | OAuth2 preferred, password fallback | yahoo |
| **iCloud** | icloud.com, me.com, mac.com | IMAP | — | Password (app-specific) | — |
| **Fastmail** | fastmail.com, fastmail.fm, messagingengine.com | JMAP (api.fastmail.com) | IMAP (imap.fastmail.com:993) | Password | — |
| **Zoho** | zoho.com, zohomail.com | IMAP | — | Password | — |
| **AOL** | aol.com | IMAP | — | Password | — |
| **GMX** | gmx.com, gmx.net, gmx.de | IMAP | — | Password | — |
| **Mail.ru** | mail.ru, inbox.ru, list.ru, bk.ru | IMAP | — | Password | — |
| **Mailo** | mailo.com, net-c.com, netc.fr | IMAP | — | Password | — |

### What changes from today

1. **Gmail added**: Currently Gmail goes through a completely separate `AddAccount.tsx` flow. Discovery should detect it and route to the Gmail API path.
2. **Microsoft gets Graph as primary**: Currently treated as IMAP-only with OAuth. Discovery should prefer Graph, offer IMAP as fallback.
3. **Fastmail gets JMAP as primary**: Currently IMAP-only. Discovery should prefer JMAP.
4. **OAuth configs embedded in discovery results**: No separate lookup needed. The `AuthMethod::OAuth2` variant carries auth_url, token_url, scopes directly.
5. **Proton Bridge removed from registry**: Proton Bridge maps `protonmail.com` to `127.0.0.1:1143`, which assumes the user has Proton Bridge installed and running locally — that's not the same kind of knowledge as "gmail.com uses imap.gmail.com." This belongs as a manual/advanced local integration path in the UI (e.g., "Connect to Proton Bridge" button with setup instructions), not as an autodiscovery result. The existing `autoDiscovery.ts` entry is kept during migration for backwards compatibility but not ported to the Rust registry.

### Extensibility

The registry is a Rust `const` array — no config file, no database. Adding a provider means adding an entry and recompiling. This is intentional: the registry is curated knowledge, not user data. For unknown providers, stages 2-5 handle discovery dynamically.

---

## Stage 2: Autoconfig XML Fetch

**Purpose**: Discover settings for providers not in our registry, using the Mozilla autoconfig standard that many providers publish.

### URLs to try (in order)

```
1. https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={email}
2. https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={email}
```

HTTP variants are intentionally excluded — the autoconfig endpoint returns credentials-adjacent information (server hostnames, auth methods) and should not be fetched over plaintext.

### XML parsing

The Mozilla autoconfig format is well-defined. We parse the subset we need:

```xml
<clientConfig version="1.1">
  <emailProvider id="example.com">
    <displayName>Example Mail</displayName>
    <domain>example.com</domain>

    <incomingServer type="imap">
      <hostname>imap.example.com</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <authentication>plain</authentication>
      <username>%EMAILADDRESS%</username>
    </incomingServer>

    <outgoingServer type="smtp">
      <hostname>smtp.example.com</hostname>
      <port>587</port>
      <socketType>STARTTLS</socketType>
      <authentication>plain</authentication>
      <username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>
```

### Fields to extract

| XML element | Maps to | Notes |
|------------|---------|-------|
| `incomingServer/@type` | `Protocol` variant | `imap` → IMAP, `exchange` → skip (use Graph) |
| `hostname` | `ServerConfig.hostname` | After variable substitution |
| `port` | `ServerConfig.port` | |
| `socketType` | `Security` | `SSL` → Tls, `STARTTLS` → StartTls, `plain` → None |
| `authentication` | `AuthMethod` | `OAuth2` → OAuth2, everything else → Password |
| `username` | `UsernameFormat` | `%EMAILADDRESS%` → EmailAddress, `%EMAILLOCALPART%` → LocalPart |
| `displayName` | `provider_name` | |
| `domain` | Used for validation | Confirm we got config for the right domain |

### Variable substitution

Autoconfig XML uses placeholder variables in hostname and username fields:

| Variable | Replacement |
|----------|------------|
| `%EMAILADDRESS%` | Full email (user@example.com) |
| `%EMAILLOCALPART%` | Local part (user) |
| `%EMAILDOMAIN%` | Domain (example.com) |

### Implementation references

Thunderbird's autoconfig XML handling is in `docs/discovery/` (gitignored, from Thunderbird commit `762ee44`):
- **`readFromXML.sys.mjs`** — full parser for the Mozilla autoconfig XML format. Covers all field extraction, variable substitution, multi-server entries, and edge cases in real-world XML.
- **`Sanitizer.sys.mjs`** — input validation for parsed values (hostname legality, port range, enum whitelisting). Good reference for what to validate before trusting network-sourced config.

### XML parser choice

Use `quick-xml` (already a transitive dependency via `jmap-client`) for SAX-style parsing. We don't need a full DOM — iterate events, extract the ~8 fields we care about. No new dependency.

### Error handling

- Non-200 response → stage produces no result (not an error).
- Malformed XML → log warning, produce no result.
- Missing required fields (hostname, port) → log warning, produce no result.
- Valid XML but `<incomingServer type="exchange">` only → skip (we handle Microsoft via Graph, not EWS).

---

## Stage 3: MX-Based Domain Resolution

**Purpose**: Handle custom domains on hosted providers. `user@company.com` with MX pointing to Google/Microsoft/Fastmail should get those providers' configs.

### Algorithm

```
1. DNS MX lookup for {domain}
2. For each MX record (by priority):
   a. Extract the base domain from the MX hostname.
      Example: mx1.smtp.messagingengine.com → messagingengine.com
   b. Check the registry for that base domain.
   c. If not found, try autoconfig XML for the MX base domain.
   d. If found, return result with source = MxLookup { mx_domain }.
3. If no MX records or no match → stage produces no result.
```

### MX → Provider examples

| MX hostname | Base domain | Registry match |
|-------------|-------------|---------------|
| `alt1.gmail-smtp-in.l.google.com` | `google.com` | → Gmail API |
| `mx.outlook.com` | `outlook.com` | → Microsoft Graph |
| `in1-smtp.messagingengine.com` | `messagingengine.com` | → Fastmail JMAP |
| `mx.zoho.com` | `zoho.com` | → Zoho IMAP |
| `mail.protonmail.ch` | `protonmail.ch` | → no match (Proton has no standard IMAP) |

### Base domain extraction

Extract the registrable domain (eTLD+1) from the MX hostname. This is the non-trivial part — you can't just take the last two segments because of multi-part TLDs like `.co.uk`, `.com.au`, `.co.jp`.

Options:
- **`addr` crate** with the public suffix list — correct but adds a dependency and needs periodic updates.
- **Heuristic**: strip the MX hostname down to the last 2 segments, with a small hardcoded exception list for known multi-part TLDs (`.co.uk`, `.co.jp`, `.com.au`, `.com.br`). Good enough for MX-to-provider mapping because we're matching against our own registry, not doing general domain parsing.
- **DNS lookup shortcut**: for our use case, we can try progressively shorter suffixes of the MX hostname against the registry. `in1-smtp.messagingengine.com` → try `messagingengine.com` → hit. This avoids needing a public suffix list entirely.

**Decision**: Use the progressive-suffix approach. Try the MX hostname with segments stripped from the left until we get a registry hit or autoconfig result. This is simple, correct for our use case, and requires no new dependencies.

### DNS resolution

Use `tokio::net::lookup_host` for simplicity, or the `hickory-resolver` (formerly `trust-dns-resolver`) crate if we need MX record parsing (which `lookup_host` doesn't provide).

MX records require an actual DNS MX query, not just A/AAAA resolution. Options:

1. **Shell out to `dig`/`nslookup`** — works but fragile.
2. **`hickory-resolver`** — full async DNS resolver with MX support. ~2.5MB binary impact. Well-maintained (Mozilla employee + community). Already tokio-native.
3. **Raw UDP DNS query** — buildable but not worth the effort.

**Decision**: Use `hickory-resolver`. MX queries are the core of stage 3, and doing DNS correctly matters. The crate is the standard Rust choice. Feature-gate it to `dns` to keep it optional if needed.

```toml
hickory-resolver = { version = "0.25", features = ["tokio"] }
```

---

## Stage 4: JMAP Well-Known Discovery

**Purpose**: Detect JMAP support via the standard mechanism defined in RFC 8620 §2.2.

### How it works

```
GET https://{domain}/.well-known/jmap
```

If this returns HTTP 200 with a JSON response, the server supports JMAP. The response is the JMAP Session Resource, which contains:
- Account IDs
- API URL
- Upload/download URLs
- Capabilities

We don't need to parse the full session — just confirm it's valid JSON with a `capabilities` field. The actual session will be established by `jmap-client` when the account is created.

### When JMAP is found

This stage produces a `ProtocolOption` with `Protocol::Jmap { session_url }`. It runs in parallel with all other stages (including autoconfig, which might also find IMAP settings for the same domain). The merge step handles deduplication and ranking.

### Relationship to Stage 1

The registry already has JMAP URLs for known providers (Fastmail). Stage 4 catches providers NOT in the registry that support JMAP — primarily self-hosted Stalwart, Cyrus, and other JMAP servers.

### What this replaces

The existing `src-tauri/src/jmap/auto_discovery.rs` — which does essentially this same thing but in isolation from the IMAP discovery path.

---

## Stage 5: Port Probing

**Purpose**: Last resort when all other stages fail. Probe common hostnames and ports to find a working IMAP/SMTP server.

### This stage is gated

Port probing starts only after a 3-second delay, giving stages 1-4 time to return results. If any higher-priority stage succeeds within that window, port probing is cancelled before it opens any sockets. This avoids unnecessary network probes for the ~90% of cases where the registry or autoconfig works.

### Hostname candidates

Generated from the email domain, tried in order:

**Incoming (IMAP)**:
```
imap.{domain}
mail.{domain}
{domain}
```

**Outgoing (SMTP)**:
```
smtp.{domain}
mail.{domain}
{domain}
```

### Port/security matrix

For each hostname, probe these port+security combinations (in preference order):

**IMAP**:
| Port | Security | Notes |
|------|----------|-------|
| 993 | TLS | Preferred — implicit encryption |
| 143 | STARTTLS | Common alternative |

**SMTP**:
| Port | Security | Notes |
|------|----------|-------|
| 587 | STARTTLS | Standard submission port |
| 465 | TLS | Legacy but widely deployed |

We do **not** probe plaintext (port 143 without STARTTLS, port 25). If a server doesn't support encrypted connections, we won't auto-discover it — the user must configure it manually. This is a security decision.

### Probe mechanism

For each hostname+port candidate:

1. Attempt TCP connect (timeout: 3 seconds).
2. If TLS port: attempt TLS handshake. If it fails, skip.
3. If STARTTLS port: read server banner, send STARTTLS command, upgrade.
4. Optionally send a protocol command to confirm it's actually the expected service:
   - IMAP: look for `* OK` banner or send `CAPABILITY`.
   - SMTP: look for `220` banner or send `EHLO`.
5. Close connection.

### Capability detection during probing

If we connect successfully and read a banner/capability response, we can detect:
- **Auth methods advertised** (from IMAP CAPABILITY or SMTP EHLO response).
- **STARTTLS support** (presence of `STARTTLS` in capabilities).

This informs the `AuthConfig` in the result. However, we do NOT detect OAuth2 from probing — OAuth2 support is only known from the registry or autoconfig XML.

### Parallelism

Probe all hostname+port combinations concurrently (up to ~6 IMAP + ~4 SMTP probes). Return the first successful pair. Cancel remaining probes on success.

---

## OAuth Integration

### How OAuth enters the discovery result

OAuth configuration is embedded directly in the `AuthMethod::OAuth2` variant of the discovery result. There is no separate "OAuth provider lookup" step — if discovery determines that OAuth is needed, all the OAuth parameters are included in the result.

### Sources of OAuth information

| Source | How OAuth is detected |
|--------|----------------------|
| **Registry** | Entries explicitly list `AuthMethod::OAuth2 { ... }` with full endpoint config |
| **Autoconfig XML** | `<authentication>OAuth2</authentication>` in the XML triggers an OAuth provider lookup |
| **MX lookup** | Resolves to a registry entry that has OAuth |
| **Port probing** | Never detects OAuth (can't be determined from a socket connection) |

### OAuth provider database

When autoconfig XML says "OAuth2" but doesn't provide endpoints (the XML format doesn't carry OAuth URLs), we need a mapping from domain → OAuth endpoints. This is a small lookup table:

```rust
struct OAuthEndpoints {
    provider_id: &'static str,
    auth_url: &'static str,
    token_url: &'static str,
    scopes: &'static [&'static str],
    use_pkce: bool,
}
```

| Domain pattern | Provider | Scopes |
|---------------|----------|--------|
| `*.google.com`, `gmail.com` | google | gmail.modify, gmail.send |
| `*.outlook.com`, `*.hotmail.com`, `*.live.com` | microsoft | IMAP.AccessAsUser.All, SMTP.Send, offline_access |
| `*.yahoo.com`, `*.ymail.com` | yahoo | mail-r, mail-w |

This is a subset of the registry — only providers we have working OAuth flows for.

### When autoconfig says OAuth2 but we lack endpoints

If autoconfig XML says `<authentication>OAuth2</authentication>` for a domain we don't have OAuth endpoints for, we do **not** silently fall back to password auth. That would prompt the user for a password that probably won't work (the provider explicitly declared it wants OAuth), producing a confusing failure.

Instead, the `AuthConfig` for that option is set to:

```rust
pub enum AuthMethod {
    Password,
    OAuth2 { /* ... */ },
    /// The provider requires OAuth2 but we don't have endpoint
    /// metadata for it. The user cannot connect via this option
    /// until we add support for this provider's OAuth flow.
    OAuth2Unsupported { provider_domain: String },
}
```

The UI should present this as: "This provider requires OAuth2 authentication, which Ratatoskr doesn't support yet for {domain}." If the same discovery run also found an IMAP option with password auth from a different stage (e.g., port probing found IMAP on a different port), that option is still available — but it's the user's informed choice, not a silent downgrade from what the provider's own autoconfig declared.

### Scopes depend on protocol

The OAuth scopes differ based on which protocol is being used:

| Provider | IMAP scopes | API scopes (Gmail/Graph) |
|----------|-------------|--------------------------|
| Google | `gmail.readonly` (not used — we prefer Gmail API) | `gmail.modify`, `gmail.send`, `gmail.readonly` |
| Microsoft | `IMAP.AccessAsUser.All`, `SMTP.Send`, `offline_access` | `Mail.ReadWrite`, `Mail.Send`, `offline_access` |

The discovery result includes the correct scopes for the **selected protocol**, not a generic set. If the user later switches from Graph to IMAP, the scopes need to change — but that's a UI concern, not a discovery concern.

### What this replaces

The TS `providers.ts` file with its separate `OAuthProviderConfig` records. OAuth knowledge moves into the Rust discovery module.

---

## Username Format Detection

Most providers use the full email address as the IMAP/SMTP username. Some don't.

### How it's detected

| Source | Username info |
|--------|--------------|
| **Registry** | Hardcoded per provider (almost all use `EmailAddress`) |
| **Autoconfig XML** | `<username>%EMAILLOCALPART%</username>` vs `<username>%EMAILADDRESS%</username>` |
| **Port probing** | Assumes `EmailAddress` (no way to detect) |

### Impact on the UI

The `UsernameFormat` is informational — it tells the UI what to pre-fill in the username field:

- `EmailAddress` → pre-fill with `user@example.com`, hide the username field (it equals the email).
- `LocalPart` → pre-fill with `user`, show the username field so the user can confirm.
- `Custom` → show the username field with the custom value, editable.

This replaces the current `imap_username` nullable column behavior. The column stays (for user overrides), but discovery now provides the default.

---

## Validation & Verification

Discovery produces a **candidate config**. Verification confirms it works before account creation.

### Existing verification

We already have Tauri commands for connection testing:
- `imap_test_connection` — connects to IMAP server, authenticates, runs CAPABILITY.
- `smtp_test_connection` — connects to SMTP server, authenticates, runs EHLO.
- `jmap_test_connection` — JMAP session discovery + auth via `jmap-client`.

### Verification flow (unchanged by this work)

1. Discovery returns `DiscoveredConfig` with one or more `ProtocolOption`s.
2. UI presents the primary option (or lets user choose).
3. User provides password (or completes OAuth).
4. UI calls the appropriate test command (`imap_test_connection`, `jmap_test_connection`, etc.).
5. If test fails → UI shows error, user can edit settings or try an alternative option.
6. If test passes → proceed to account creation.

### What discovery does NOT do

- Does **not** test authentication. That requires user credentials, which aren't available during discovery.
- Does **not** validate TLS certificates (other than the autoconfig HTTPS fetch). Certificate validation happens during the connection test.
- Does **not** verify that the server is actually operational. A port probe confirms something is listening, but it might be misconfigured.

Discovery finds the door. Verification opens it.

---

## Tauri Commands

### Primary command

```rust
#[tauri::command]
pub async fn discover_email_config(email: String) -> Result<DiscoveredConfig, String>
```

Takes an email address, runs the full cascade, returns the merged result. This is the only command the TS side needs.

### Optional diagnostic command

```rust
#[tauri::command]
pub async fn discover_email_config_verbose(
    email: String,
) -> Result<DiscoveryDiagnostics, String>
```

Returns per-stage results with timing, for debugging in settings/advanced. Not needed for the initial implementation — add when useful.

---

## TS Integration

### Account setup flow (target)

```
1. User enters email address
2. TS calls discover_email_config(email)
3. Result determines which UI path:
   a. GmailApi → redirect to Gmail OAuth flow (existing AddAccount.tsx)
   b. MicrosoftGraph → redirect to Microsoft OAuth flow (new)
   c. Jmap → show JMAP setup (URL pre-filled, password or OAuth)
   d. Imap → show IMAP/SMTP setup (pre-filled from discovery)
4. If multiple options → show picker ("Connect via JMAP (recommended)" / "Connect via IMAP")
5. User provides credentials → test connection → create account
```

### What changes in the UI

- **Unified entry point**: One "Add Account" flow that starts with email entry. No separate "Add Gmail Account" vs "Add IMAP Account" buttons.
- **Protocol picker**: When discovery returns multiple options, show a simple selector. Default to the highest-ranked option.
- **Auto-filled fields**: IMAP/SMTP host, port, security are pre-filled and hidden by default. Show an "Advanced" toggle to reveal them for manual editing.
- **OAuth auto-trigger**: If the best option requires OAuth2 and only OAuth2 (no password fallback), start the OAuth flow immediately after discovery completes.

### What stays the same

- The existing `imap_test_connection`/`smtp_test_connection`/`jmap_test_connection` commands.
- The existing `insertImapAccount`/`insertOAuthImapAccount` DB functions.
- The existing OAuth flow in `oauthFlow.ts` (just called with different parameters from discovery).
- The `accountStore` and `syncManager` integration.

---

## File Structure

```
src-tauri/src/discovery/
├── mod.rs              // Module root, re-exports, cascade orchestration
├── types.rs            // DiscoveredConfig, ProtocolOption, AuthConfig, etc.
├── registry.rs         // Stage 1: hardcoded provider database
├── autoconfig.rs       // Stage 2: Mozilla autoconfig XML fetch + parse
├── mx.rs               // Stage 3: DNS MX lookup + domain re-resolution
├── jmap_wellknown.rs   // Stage 4: .well-known/jmap probe
├── probe.rs            // Stage 5: port probing
├── commands.rs         // Tauri command wrappers
└── merge.rs            // Result merging, deduplication, ranking
```

### Relationship to existing code

| New file | Replaces |
|----------|----------|
| `discovery/registry.rs` | `src/services/imap/autoDiscovery.ts` (wellKnownProviders) |
| `discovery/registry.rs` | `src-tauri/src/jmap/auto_discovery.rs` (KNOWN_PROVIDERS) |
| `discovery/registry.rs` | `src/services/oauth/providers.ts` (OAuth endpoint configs) |
| `discovery/jmap_wellknown.rs` | `src-tauri/src/jmap/auto_discovery.rs` (.well-known probe) |

The TS files are deleted after migration. The JMAP `auto_discovery.rs` is absorbed into the new module.

---

## What We Skip

### Exchange AutoDiscover (EWS/EAS)

Thunderbird implements full Exchange AutoDiscover with XML POST bodies, redirect chains, and SRV lookups. We skip this entirely because:
- Our Microsoft path is Graph API, not EWS/EAS.
- Microsoft Graph uses standard OAuth2, not AutoDiscover.
- Outlook.com domains are in our registry; custom domains are caught by MX lookup.

### Mozilla ISPDB (central database)

Thunderbird queries Mozilla's central ISP database as a fallback. This is Mozilla-specific infrastructure we can't rely on. Our stages 2 (autoconfig XML) and 3 (MX lookup) cover the same ground via different means.

### HTTP autoconfig URLs

Thunderbird tries HTTP (not just HTTPS) variants of autoconfig URLs. We only try HTTPS. The autoconfig response contains security-sensitive information (server hostnames, auth methods) that shouldn't traverse the network in plaintext, even if it's not credentials.

### SRV DNS records

Some providers publish `_imap._tcp.{domain}` or `_submission._tcp.{domain}` SRV records. Thunderbird uses SRV for Exchange AutoDiscover but not for IMAP/SMTP discovery. We skip SRV for now — autoconfig XML and port probing cover the same use case with better reliability. SRV could be added as a stage between MX lookup and port probing if needed later.

### Client certificate authentication

Thunderbird supports `<authentication>TLS-client-cert</authentication>`. Rare in consumer email. Skip.

### NNTP (Usenet)

Not an email protocol. Skip.

---

## Reference: Thunderbird's Approach

Source: `docs/discovery/` (Thunderbird `mail/components/accountcreation/modules/` at commit `762ee44`).

### What we adopted from Thunderbird

1. **Autoconfig XML format**: The standard Mozilla autoconfig XML that many providers host. Same URLs, same XML schema, same variable substitution.
2. **MX-based domain resolution**: Their `ForMX` stage is our Stage 3. The insight that MX records reveal the actual hosting provider is the key to handling custom domains.
3. **Port probing hostname/port matrix**: Their `GuessConfig.sys.mjs` establishes the standard hostname patterns (`imap.{domain}`, `smtp.{domain}`, `mail.{domain}`) and port preference order. We use the same matrix, minus plaintext.
4. **Parallel execution**: Their `promiseFirstSuccessful` pattern inspired our concurrent stage design. We diverge on cancellation — Thunderbird takes the first success and cancels the rest (it only needs one config), while we collect all results (we want multiple protocol options). But the core idea of running all stages concurrently with timeouts is the same.
5. **OAuth2 detection from autoconfig**: When XML says `OAuth2`, look up the provider's OAuth endpoints. Skip port probing entirely if OAuth is detected (no point probing if we know the auth method).
6. **Username variable substitution**: `%EMAILADDRESS%` vs `%EMAILLOCALPART%` in autoconfig XML tells us the username format without guessing.

### What we diverged on

1. **No local disk configs**: Thunderbird ships ISP XML files for managed deployments. We're a consumer app, not an enterprise tool.
2. **No Exchange AutoDiscover**: We use Microsoft Graph, not EWS/EAS.
3. **No Mozilla ISPDB**: Third-party infrastructure dependency.
4. **HTTPS-only autoconfig**: Thunderbird falls back to HTTP. We don't.
5. **No plaintext port probing**: Thunderbird probes port 143 without STARTTLS and port 25 without STARTTLS. We require encryption.
6. **Multi-protocol results**: Thunderbird finds one config. We find multiple (JMAP + IMAP, Graph + IMAP) and let the user choose. This is fundamental to our multi-protocol architecture.
7. **JMAP as a first-class discovery target**: Thunderbird has no JMAP support. Our Stage 4 adds `.well-known/jmap` probing, which is trivial but essential.

---

## Open Questions

### 1. Should `hickory-resolver` be a required or optional dependency?

MX lookup (Stage 3) requires proper DNS MX queries. `hickory-resolver` is the standard Rust crate for this (~2.5MB binary impact). Alternative: skip MX lookup entirely and rely on autoconfig + port probing for unknown domains. This would mean custom domains on hosted providers only work if the host publishes autoconfig XML.

**Leaning**: Required. MX lookup is the single most impactful discovery method for the long tail of custom domains on major hosts.

### 2. Should port probing use our existing `async-imap` / `lettre` or raw sockets?

Using `async-imap` for IMAP probing would give us proper protocol handling (CAPABILITY parsing, STARTTLS negotiation) but requires constructing a full client. Raw TCP + minimal protocol parsing is lighter but duplicates some logic.

**Leaning**: Raw sockets with minimal banner parsing. We just need to confirm the service is there, not establish a full session. The connection test commands handle full protocol validation.

### 3. When should the TS `autoDiscovery.ts` and `oauth/providers.ts` be deleted?

After the Rust discovery module is wired up and the unified account setup UI is working. This is a Phase 2 (TS integration) concern. During development, both can coexist — the TS side calls the Rust command when available, falls back to the TS implementation otherwise.

### 4. Should discovery results be cached?

Thunderbird doesn't cache. Discovery is fast enough to run on every account setup attempt. But if we add discovery to other flows (e.g., contact hover cards showing provider info), caching might matter.

**Leaning**: No cache initially. Revisit if discovery is called frequently.

### 5. How do we handle providers that require app-specific passwords?

iCloud and some others require app-specific passwords, not the user's account password. Discovery can detect this (iCloud is in our registry) and show a help link, but it can't generate the app password. This is a UX concern, not a discovery concern.

**Leaning**: Registry entries can include a `help_url` field pointing to the provider's app password generation page. The UI shows this when relevant.
