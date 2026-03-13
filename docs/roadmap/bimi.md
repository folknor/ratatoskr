# BIMI (Brand Indicators for Message Identification)

**Tier**: 3 — Differentiators and polish
**Status**: ❌ **Not implemented**

---

- **What**: Verified sender brand logos displayed next to messages from authenticated domains
- **Scope**: Client-side only — DNS lookup + header check, works identically across all providers

## Pain points

- Performance: every unique sender domain requires a DNS TXT lookup + potential SVG fetch. At hundreds of emails/day with diverse senders, this is a lot of lookups. Need aggressive caching per domain (logos don't change often — cache for days/weeks).
- Validation: BIMI requires DMARC pass. Must check `Authentication-Results` header for DMARC status. If the header is missing or DMARC failed, don't display the logo (it's unverified).
- SVG rendering: BIMI logos are SVG Tiny PS (a restricted SVG profile). Need an SVG renderer that handles this subset. Full SVG renderers may work, but the spec is specific about what's allowed.
- VMC (Verified Mark Certificate): full BIMI validation requires checking a VMC certificate (X.509 with the logo embedded). This is the "verified" part. Without VMC checking, you can still display the logo but can't claim it's verified. VMC checking is complex (certificate chain validation, embedded logo comparison). Start without VMC, add later.
- Fallback: for domains without BIMI, fall back to colored initials or gravatar. BIMI should feel like an enhancement over the default avatar, not a required element.

## Work

DNS BIMI record lookup, SVG logo fetch + cache per domain, check DMARC pass in `Authentication-Results`, display in sender avatar slot. Skip VMC validation initially.

---

## Research

**Date**: March 2026
**Context**: Evaluating implementation options for BIMI in the iced (pure Rust) rewrite. Client-side feature, works identically across all four providers since it only depends on message headers and DNS.

---

### 1. BIMI Protocol Mechanics

BIMI is defined in [draft-brand-indicators-for-message-identification-12](https://datatracker.ietf.org/doc/html/draft-brand-indicators-for-message-identification) (November 2025), still an IETF draft. Stable enough that Gmail, Apple Mail, Yahoo, and Fastmail all implement it in production.

#### DNS Record Format

TXT record at `{selector}._bimi.{domain}` (selector defaults to `default`):

```
default._bimi.example.com. IN TXT "v=BIMI1; l=https://example.com/logo.svg; a=https://example.com/cert.pem"
```

Tags:
- **`v=BIMI1`** (required, must be first)
- **`l=`** (required) — HTTPS URI to SVG Tiny PS logo. Empty = explicit opt-out
- **`a=`** (optional) — HTTPS URI to VMC/CMC evidence document (PEM). Empty = no certificate
- Fallback: if no record at `{selector}._bimi.{from-domain}`, try organizational domain

#### BIMI-Selector Header

Senders specify non-default selector via `BIMI-Selector: v=BIMI1; s=marketing`, causing lookup at `marketing._bimi.{domain}`. Must be DKIM-signed; if unsigned, fall back to `default`.

#### BIMI-Location and BIMI-Indicator Headers

**Receiver-injected headers**, not sender headers:
- Senders MUST NOT set these
- Receivers strip pre-existing `BIMI-*` headers before processing
- After validation, receiving MTA inserts:
  - **`BIMI-Location`**: `v=BIMI1; l=<svg-uri>`
  - **`BIMI-Indicator`**: Base64-encoded SVG content (ready for display)

**Critical for implementation**: If the receiving server already did BIMI validation, `BIMI-Indicator` is present — skip DNS lookups, just decode. For IMAP servers that don't strip BIMI headers, an attacker could inject fakes. Trust model:
- Gmail API / JMAP (Fastmail) / Graph (Outlook): headers trustworthy
- Generic IMAP: headers NOT trustworthy — do own validation or ignore pre-existing

#### Full Validation Chain (for generic IMAP)

1. Check `Authentication-Results` for `dmarc=pass`
2. Check domain DMARC policy is `p=quarantine;pct=100` or `p=reject`
3. DNS lookup `{selector}._bimi.{domain}` TXT
4. Fallback to `default._bimi.{organizational-domain}`
5. HTTPS fetch SVG from `l=` URI
6. Validate SVG Tiny PS compliance, file size <=32KB
7. (Optional) Validate VMC/CMC from `a=` URI

---

### 2. DNS Lookup in Rust

#### hickory-resolver (formerly trust-dns-resolver)

- **Crate**: [hickory-resolver](https://crates.io/crates/hickory-resolver) v0.26.0-alpha.1
- **Downloads**: ~38M total, ~10.7M recent
- **Maintainer**: hickory-dns org, 5.1K GitHub stars
- **Last updated**: June 2025

De facto standard DNS resolver in Rust. Pure Rust, fully async (tokio), supports all record types including TXT. Built-in TTL-based caching. Concurrent lookups via `join_all`.

#### dns-lookup

- **Downloads**: ~18.8M total
- Thin libc wrapper. Synchronous only. **Cannot do TXT record lookups** — only A/AAAA. Not viable.

#### c-ares-resolver

- **Downloads**: ~217K total
- C `c-ares` bindings. Supports TXT, async. But adds C dependency with minimal benefit over hickory-resolver.

**Recommendation**: Use hickory-resolver. Dominant, pure Rust, TTL caching built-in.

---

### 3. SVG Rendering in Rust

#### SVG Tiny PS Profile

Security-focused SVG subset. **Forbidden**: scripts, hyperlinks, foreignObject, animations, external references, event handlers. **Allowed**: basic shapes, fills, strokes, gradients, transforms, grouping, basic text, clipping, opacity. Must include `baseProfile="tiny-ps"` and `version="1.2"`. Max 32KB. Square aspect ratio.

#### resvg

- **Crate**: [resvg](https://crates.io/crates/resvg) v0.47.0
- **Downloads**: ~10.7M total, ~3M recent
- **Maintainer**: linebender org, 3.7K stars
- **Last updated**: February 2026
- **Dependencies**: `usvg` (parsing), `tiny-skia` (rasterizer)

Only serious SVG renderer in Rust. Pure Rust, no system deps. Targets SVG 1.1 Full. Does not explicitly support SVG Tiny 1.2 as a profile, but SVG Tiny PS is a strict subset of SVG 1.1 — any valid BIMI logo renders correctly.

**Security**: Already strips scripts, ignores event handlers. Fuzz-tested with ~1600 regression tests. However, will follow external references. Pre-validate SVG before passing to resvg: check `baseProfile`, reject external URI references, enforce 32KB limit.

#### Rendering Pipeline for iced

1. Fetch SVG bytes (from URI or decode `BIMI-Indicator` header)
2. Validate SVG Tiny PS compliance
3. Parse with `usvg::Tree::from_data()`
4. Render with `resvg::render()` to `tiny_skia::Pixmap`
5. Convert to `iced::widget::image::Handle` via `Handle::from_rgba()`
6. Display with iced `Image` widget (pre-rasterized, not per-frame SVG rendering)

---

### 4. DMARC Validation from Headers

#### Authentication-Results parsing

Format (RFC 8601): `authserv-id; method=result (comment) property=value; ...`

For BIMI, need `dmarc=pass` from trusted authserv-id.

**No Rust crate exists for parsing `Authentication-Results`.** Neither `mail-parser` nor `mailparse` provide structured parsing — raw string only.

For BIMI's narrow needs, simple string matching suffices. A full RFC 8601 parser (using `nom` or `winnow`) is optional hardening.

#### Trust model

- Gmail API, JMAP (Fastmail), Graph: trust `Authentication-Results`
- Generic IMAP: trust if authserv-id matches connected server. If no header or unrecognizable, skip BIMI.

---

### 5. VMC / CMC Certificate Validation

#### What they are

Both are X.509 certificates binding a brand logo to a domain:
- **VMC**: Requires registered trademark. Gmail shows blue checkmark.
- **CMC**: No trademark needed (12 months documented logo use). No checkmark.

Both embed SVG in the certificate's logotype extension (OID 1.3.6.1.5.5.7.1.12, RFC 3709).

#### Certificate validation crates

| Crate | Downloads | Pure Rust | Parse extensions | Chain validation |
|---|---|---|---|---|
| `x509-parser` 0.18.1 | 91M / 20M recent | Yes | Yes (arbitrary) | No |
| `rustls-webpki` 0.104 | 423M / 98M recent | Yes | No (Web PKI only) | Yes |
| `openssl` 0.10.76 | 264M / 34M recent | No (C FFI) | Yes | Yes |

**For VMC (Phase 2)**: `x509-parser` for certificate parsing + logotype extraction, `rustls-webpki` for chain validation against MVA root certificates. Avoids OpenSSL C dependency.

#### Should we implement VMC early?

**No.** Logo display works without VMC (gated on DMARC pass + DNS lookup). Gmail/Fastmail already do VMC server-side. For generic IMAP, most senders won't have VMCs. Phase 2 feature.

---

### 6. Caching Architecture

#### Cache layers

1. **DNS cache** (automatic via hickory-resolver TTL caching)
2. **BIMI result cache** (per-domain, 24-48h): has BIMI / no BIMI / logo URI
3. **SVG/bitmap cache** (per-logo URI): rasterized bitmap, invalidate when URI changes
4. **Negative cache** (24-48h): domains without BIMI — highest-impact layer, prevents wasted lookups for the majority of domains

#### Storage

SQLite table:
```sql
CREATE TABLE bimi_cache (
    domain        TEXT PRIMARY KEY,
    has_bimi      INTEGER NOT NULL,
    logo_uri      TEXT,
    authority_uri TEXT,
    fetched_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL
);
```

Filesystem for bitmaps: `{app_data}/bimi/{sha256(logo_uri)}.png` (~5-15KB at 128x128). In-memory LRU (N=500) on top for active rendering.

#### Cache warming

Collect unique sender domains from visible messages, filter against cache, batch concurrent DNS lookups (concurrency limit ~20), fetch SVGs, rasterize and cache. Completes in 1-2 seconds for typical page. UI renders with fallback avatars, BIMI logos pop in as they resolve.

#### BIMI-Indicator shortcut

For Gmail, JMAP (Fastmail): decode base64 SVG from header, rasterize, cache by domain. No DNS, no HTTP. Covers majority of users.

---

### 7. What Other Clients Do

| Client | BIMI | VMC Required? | Notes |
|---|---|---|---|
| Gmail | Full (2021) | VMC for checkmark; CMC for logo | Server-side. Injects `BIMI-Indicator`. |
| Apple Mail | iOS 16+ | VMC required | Server-side via iCloud only |
| Yahoo Mail | Full | No VMC needed | Early adopter |
| Fastmail | Full (2021) | No VMC needed | BIMI logos display even with remote images blocked |
| Thunderbird | **Not supported** | N/A | Blocker: can't trust BIMI headers from arbitrary IMAP servers |
| Outlook | **Not supported** | N/A | Microsoft hasn't implemented BIMI |

**Key insight**: Every supporting client is tightly integrated with a specific mail server. No arbitrary-IMAP client has implemented BIMI. Our advantage: we know which server processed each message across our 4 provider types.

---

### 8. Comparison Matrices

#### DNS Resolver Crates

| | hickory-resolver | dns-lookup | c-ares-resolver |
|---|---|---|---|
| Downloads | 38M / 10.7M recent | 18.8M / 5.5M | 217K / 7K |
| Pure Rust | Yes | No (libc) | No (c-ares) |
| Async | tokio | No | tokio/futures |
| TXT records | Yes | **No** | Yes |
| Built-in cache | Yes (TTL) | No | No |
| **Verdict** | **Use this** | Cannot do TXT | Unnecessary |

#### SVG Rendering

| | resvg + usvg | iced Svg widget |
|---|---|---|
| Downloads | resvg 10.7M, usvg 11.7M | (part of iced) |
| Pre-rasterize | Yes — Pixmap to Handle | No — per-frame |
| Validation hook | Yes — inspect tree | No |
| **Verdict** | **Use this** | Less control |

#### Certificate Validation (Phase 2)

| | x509-parser | rustls-webpki | openssl |
|---|---|---|---|
| Downloads | 91M / 20M | 423M / 98M | 264M / 34M |
| Pure Rust | Yes | Yes | No |
| Parse extensions | Yes | No | Yes |
| Chain validation | No | Yes | Yes |
| **Verdict** | Parsing + extraction | Chain verification | Avoid (C dep) |

---

### Implementation Plan

**Phase 1 (ship with iced MVP)**:
- Parse `Authentication-Results` for `dmarc=pass`
- Check `BIMI-Indicator` header first (covers Gmail, Fastmail)
- DNS lookup via hickory-resolver, SVG fetch via reqwest
- SVG Tiny PS validation before rendering
- Render with resvg to bitmap, display in iced avatar slot
- SQLite + filesystem caching with negative caching

**Phase 2 (post-launch)**:
- VMC/CMC validation using x509-parser + rustls-webpki
- "Verified" badge for VMC-validated logos
- DMARC policy DNS lookup (verify p=quarantine/reject ourselves)

**Dependencies to add**: `hickory-resolver` (DNS), `resvg` + `usvg` (SVG), `reqwest` (HTTP). Phase 2: `x509-parser`.
