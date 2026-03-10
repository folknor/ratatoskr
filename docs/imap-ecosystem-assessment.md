# Rust IMAP & JMAP Ecosystem Assessment

**Date**: March 2026
**Context**: Ratatoskr uses `async-imap` v0.11 with a 477-line raw TCP fallback (`src-tauri/src/imap/raw.rs`) to work around parser failures on non-standard IMAP servers. This document evaluates whether better alternatives exist.

**Note**: The IMAP crate ecosystem analysis below remains current. The JMAP and Microsoft Graph recommendations in the final sections are outdated — both providers were subsequently implemented in Rust. See `docs/jmap-rust-migration.md` and `docs/graph-rust-migration.md` for completed summaries.

---

## Table of Contents

- [Current Implementation](#current-implementation)
- [The imap-proto Foundation](#the-imap-proto-foundation)
- [Crate-by-Crate Assessment](#crate-by-crate-assessment)
  - [async-imap (current)](#async-imap-current)
  - [rust-imap](#rust-imap)
  - [imap-next + imap-codec (duesee)](#imap-next--imap-codec-duesee)
  - [imap-client / io-imap (Pimalaya)](#imap-client--io-imap-pimalaya)
  - [email-lib (Pimalaya)](#email-lib-pimalaya)
  - [tokio-imap](#tokio-imap)
  - [melib](#melib)
- [Comparison Matrix](#comparison-matrix)
- [IMAP4rev2 Support](#imap4rev2-support)
- [JMAP as an Alternative](#jmap-as-an-alternative)
  - [Protocol Overview](#protocol-overview)
  - [Provider Support](#provider-support)
  - [Rust JMAP Crates](#rust-jmap-crates)
  - [JMAP Verdict](#jmap-verdict)
- [Non-Rust Alternatives (FFI)](#non-rust-alternatives-ffi)
- [Recommendations](#recommendations)

---

## Current Implementation

Our IMAP layer is ~1,967 lines across 5 modules in `src-tauri/src/imap/`:

| Module | Lines | Purpose |
|--------|-------|---------|
| `client.rs` | 772 | Public API facade — thin wrappers around async-imap with timeouts and error formatting |
| `connection.rs` | 302 | TCP/TLS/STARTTLS setup, XOAUTH2 authenticator, socket tuning |
| `parse.rs` | 306 | Bridge between `mail-parser` output and IMAP section addressing for attachments |
| `raw.rs` | 477 | **Raw TCP fallback** — bypasses async-imap entirely for servers with non-standard responses |
| `types.rs` | 105 | Serde structs for Tauri IPC |

The raw TCP fallback exists because `async-imap`'s parser (based on `imap-proto`) fails on servers like Mailo that send flags without backslash prefixes (e.g., `Sent` instead of `\Sent`). Rather than silently handling these deviations, the parser panics or returns errors.

The message parsing layer (`parse.rs`) is necessary regardless of IMAP library choice — `async-imap` returns raw RFC 2822 bytes, and `mail-parser` handles MIME decoding. The custom code bridges the two by mapping `mail-parser`'s flat part vector to IMAP section paths (e.g., `1.2.3`) needed for attachment download.

---

## The imap-proto Foundation

[`imap-proto`](https://crates.io/crates/imap-proto) (v0.16.6, Sep 2025) is the protocol parser that underpins both `async-imap` and `rust-imap`. It is maintained by Dirkjan Ochtman (djc) in low-frequency mode (~1 release/year).

**Critical limitations:**

1. **No IMAP4rev2 (RFC 9051) support.** The [`Capability` enum](https://docs.rs/imap-proto/0.16.6/imap_proto/types/enum.Capability.html) has only three variants: `Imap4rev1`, `Auth(Cow<str>)`, `Atom(Cow<str>)`. A server advertising `IMAP4rev2` silently falls into the `Atom` catch-all. There is no protocol-level support for rev2 behavioral changes (mandatory CONDSTORE, new FETCH semantics, STATUS=SIZE, LOGIN removal, etc.).

2. **Strict nom parser.** The nom-based parser follows RFC 3501 grammar strictly with no tolerance for real-world deviations. Servers that send non-standard flags, malformed BODYSTRUCTURE responses, or non-UTF8 data cause parse failures. Open issues for BODYSTRUCTURE parsing (#15) and non-UTF8 data (#5) date to 2017 and remain unfixed.

3. **No roadmap.** Nobody has filed an issue or PR for IMAP4rev2 support. The crate receives minimal maintenance — enough to keep it compiling, not enough to evolve it.

Any IMAP crate built on `imap-proto` inherits these limitations. This includes both `async-imap` and `rust-imap`.

---

## Crate-by-Crate Assessment

### async-imap (current)

- **Crate**: [async-imap](https://crates.io/crates/async-imap) v0.11.2
- **Maintainer**: link2xt (Delta Chat team, [chatmail/async-imap](https://github.com/chatmail/async-imap) fork)
- **Downloads**: ~305K total
- **Parser**: `imap-proto` (nom-based, see limitations above)

**What it provides**: LIST, SELECT, UID FETCH, UID SEARCH, UID STORE, UID MOVE, UID COPY, EXPUNGE, APPEND, STATUS, IDLE, AUTHENTICATE. Async with tokio or async-std via feature flags.

**What it doesn't provide**: Message/MIME parsing (raw bytes only), connection lifecycle management, granular timeouts, XOAUTH2 (must implement `Authenticator` trait yourself), CONDSTORE/QRESYNC, IMAP4rev2.

**Known issues**:
- Parser breaks on non-standard servers (Mailo, some Exchange configs, various European providers) — this is why we have `raw.rs`
- STARTTLS requires manual stream upgrade (connect plain, issue STARTTLS, wrap in TLS, rebuild Client)
- No built-in timeout management — must wrap every call with `tokio::time::timeout`
- Only supports LOGIN authentication mechanism, not PLAIN — this contributes to issue #204 (goneo.de auth failure)

**Verdict**: Functional but brittle. The Delta Chat team has the most real-world multi-provider IMAP experience in the Rust ecosystem, which is why this fork is the best maintained. But the `imap-proto` parser is a fundamental limitation.

### rust-imap

- **Crate**: [imap](https://crates.io/crates/imap) v2.4.1
- **Maintainer**: Jon Gjengset (jonhoo) — **actively seeking new maintainers**
- **Downloads**: ~640K total
- **Parser**: `imap-proto` (same limitations)

Synchronous blocking I/O only. Better API than async-imap (dedicated `connect_starttls()`, `wait_with_timeout()` for IDLE), but would require `spawn_blocking` in Tauri's async context. Same underlying parser weakness. Maintenance situation is precarious.

**Verdict**: More mature API but blocking-only and uncertain future. Not a viable migration target.

### imap-next + imap-codec (duesee)

- **Crates**: [imap-next](https://crates.io/crates/imap-next) v0.3.4, [imap-codec](https://crates.io/crates/imap-codec) v2.0.0-alpha.7
- **Maintainer**: Damian Poddebniak (duesee), funded by NLnet/NGI Assure (concluded 2024)
- **Downloads**: imap-next ~20K, imap-codec ~50K
- **Parser**: `imap-codec` (independent from `imap-proto`, fuzz-tested, zero-copy)

**Architecture**: Sans-I/O design — `imap-codec` handles parsing/serialization, `imap-next` manages protocol state machines, you bring your own I/O (tokio, async-std, or sync). This is architecturally sound but means you must build your own connection management, TLS handling, and high-level client API.

**The good**:
- Fuzz-tested parser with type-system enforced correctness
- `quirk_` feature flags for handling non-standard servers (principled workaround mechanism)
- CONDSTORE/QRESYNC support via feature flags — significant advantage for efficient delta sync
- IDLE as a first-class protocol flow

**The bad** (confirmed by examining GitHub issues and real-world usage):

1. **Himalaya issue #641** (filed March 8, 2026): **Gmail SELECT crashes** with "Received malformed message" because Gmail sends keyword flags with spaces. The strict parser chokes on the world's largest IMAP provider.

2. **Only 3 reverse dependencies** for imap-codec (imap-next, melib, protonmail-client). Only 1 for imap-next (imap-client). Almost nobody uses these in the wild.

3. **No IMAP4rev2 support.** README explicitly says "complete formal syntax of IMAP4rev1" only.

4. **MOVE extension (RFC 6851) not in the feature flags** — a widely-used extension that most modern servers support.

5. **Perpetually pre-1.0.** imap-codec went from 1.0 to 2.0-alpha, suggesting the author wasn't confident in the 1.0 API.

6. **Two-person project** (duesee + soywod). NLnet funding ended. If either person loses interest, the ecosystem stops.

7. Additional Himalaya IMAP bugs: UIDVALIDITY=0 crashes (WinWebMail), BINARY extension failures (STRATO), Outlook.com auth problems, "command not permitted with UID" when moving messages.

**Verdict**: Better architecture than async-imap, but not production-ready for multi-provider use. Crashing on Gmail SELECT is disqualifying. Check back in 6-12 months.

### imap-client / io-imap (Pimalaya)

- **Crate**: [imap-client](https://crates.io/crates/imap-client) v0.2.3 / [io-imap](https://github.com/pimalaya/io-imap)
- **Maintainer**: Clement Douin (soywod, Pimalaya/Himalaya)
- **Downloads**: ~14K total

This is the high-level async wrapper around imap-next, intended to be the "just use this" crate.

**The state of io-imap's GitHub issues is damning: 16 open issues, zero closed — ever.** Every issue filed remains open:

- **#4**: Response routing may silently swallow unsolicited responses. The maintainer wrote "this design is still (a bit) off. Need to investigate :-("
- **#11**: No public access to unsolicited responses (EXPUNGE, EXISTS notifications) — critical for any real IMAP client
- **#10**: `starttls()` doesn't check STARTTLS capability before attempting upgrade
- **#1**: XOAuth2 SASL flow not fully handled
- **#13**: Maintainer wants to refactor the entire Client using a sans-I/O approach — the current architecture is acknowledged as wrong

**Verdict**: Fundamental architectural questions remain unresolved. Not ready for production use.

### email-lib (Pimalaya)

- **Crate**: [email-lib](https://crates.io/crates/email-lib) v0.26+
- **Type**: Full email management framework (IMAP, SMTP, Maildir, Sendmail)

Powers Himalaya CLI. Too opinionated and heavy for embedding in a Tauri app that already has its own service layer. Inherits all imap-next/imap-codec limitations.

**Verdict**: Not a fit — we need a library, not a framework.

### tokio-imap

- **Crate**: [tokio-imap](https://crates.io/crates/tokio-imap)
- **Maintainer**: djc — **explicitly recommends evaluating async-imap instead**

Dead. Skip.

### melib

- **Crate**: [melib](https://lib.rs/crates/melib) v0.8.13
- **License**: GPLv3

Full mail-client library backing the `meli` terminal email client. IMAP, NNTP, Maildir, mbox backends with built-in envelope parsing. GPLv3 license is incompatible with our project.

**Verdict**: Not viable due to license.

---

## Comparison Matrix

| | async-imap | rust-imap | imap-next | imap-client | email-lib |
|---|---|---|---|---|---|
| **Downloads** | 305K | 640K | 20K | 14K | — |
| **Async** | tokio/async-std | No (sync) | Sans-I/O | tokio | tokio |
| **Parser** | imap-proto (brittle) | imap-proto (brittle) | imap-codec (strict) | imap-codec | imap-codec |
| **IMAP4rev2** | No | No | No | No | No |
| **QRESYNC** | No | No | Yes (feature flag) | Inherits | Inherits |
| **MOVE** | uid_mv() | Not first-class | Not in feature flags | Inherits | Inherits |
| **XOAUTH2** | Manual Authenticator | Manual Authenticator | Not fully handled | Not fully handled | Via config |
| **Gmail works?** | Yes (with workarounds) | Yes (with workarounds) | **Crashes on SELECT** | Inherits crash | Inherits crash |
| **Maintenance** | Active (Delta Chat) | Seeking maintainer | Active (2 people) | 0 closed issues | Active |
| **Maturity** | Stable | Stable | Pre-1.0 alpha | WIP v0.2 | v0.26 |

**No Rust IMAP crate supports IMAP4rev2.** For comparison, Go's [go-imap](https://github.com/emersion/go-imap) v2 by Simon Ser (emersion) fully supports IMAP4rev2 and has a mature ecosystem. The Go IMAP world is years ahead.

---

## IMAP4rev2 Support

IMAP4rev2 (RFC 9051, published August 2021) is the first major revision of the IMAP protocol since RFC 3501 (2003). Key changes:

- **CONDSTORE is mandatory** — every server must support change tracking
- **UTF8=ACCEPT** replaces the old internationalization approach
- **LOGIN command removed** — AUTHENTICATE is required
- **New FETCH semantics** — PREVIEW, SAVEDATE, EMAILID, THREADID
- **STATUS=SIZE** — mailbox size without fetching all messages
- **LITERAL+/LITERAL-** — non-synchronizing literals mandatory
- **Simplified capability negotiation**

No Rust IMAP crate — neither imap-proto, imap-codec, nor any higher-level library — supports IMAP4rev2 as of March 2026. The `imap-proto` `Capability` enum doesn't even have a variant for it. Adding rev2 support would require parser changes, new response codes, modified FETCH data items, updated type definitions, and careful rev1/rev2 negotiation handling.

This is a significant gap. Major servers (Dovecot 2.4+, Cyrus 3.10+) are starting to advertise IMAP4rev2 capabilities.

---

## JMAP as an Alternative

### Protocol Overview

JMAP (JSON Meta Application Protocol) is a set of IETF standards designed as a modern replacement for IMAP+SMTP:

- **RFC 8620** (2019): Core protocol — session resource, get/set/query methods, change tracking, blob handling over HTTP
- **RFC 8621** (2019): JMAP for Mail — Email, Mailbox, Thread, EmailSubmission, VacationResponse
- **RFC 8887**: JMAP over WebSocket for real-time push

| Aspect | IMAP | JMAP |
|--------|------|------|
| Transport | Custom TCP protocol, stateful | HTTP + JSON, stateless |
| Push | IDLE (single folder, persistent connection) | EventSource / WebSocket (all mailboxes) |
| Send + receive | Requires separate SMTP connection | Unified EmailSubmission |
| Request batching | Not supported | Multiple methods per request with back-references |
| Mobile-friendly | Poor — connection drops, high battery | Designed for intermittent connectivity |
| Sync model | UID-based, UIDVALIDITY invalidation | State tokens (like Gmail History API) |
| Scope | Mail only | Extensible: mail, contacts, calendars, file storage |

JMAP's sync model is particularly elegant: the server returns a `state` string, and the client requests only changes since that state — standardizing what Gmail's History API does proprietarily.

### Provider Support

This is JMAP's fatal weakness as of March 2026:

| Provider | JMAP Support |
|----------|-------------|
| **Fastmail** | Yes (production, primary champion) |
| Gmail / Google | No (uses proprietary Gmail API) |
| Outlook / Microsoft | No (uses Microsoft Graph API) |
| Yahoo | No |
| iCloud / Apple | No |
| AOL, Zoho, GMX | No |

**Self-hosted servers with JMAP**: Stalwart Mail Server (Rust), Cyrus IMAP 3.x, Apache James. These serve a niche audience.

**Client adoption**: Fastmail's own apps, Thunderbird (actively implementing), meli (optional backend), Ltt.rs (Android).

Google and Microsoft have no incentive to support an open protocol that would commoditize their email backends. This creates a chicken-and-egg deadlock that has persisted for 10+ years.

### Rust JMAP Crates

| Crate | Type | Mail | Contacts/Cal | Async | Status | License |
|-------|------|------|-------------|-------|--------|---------|
| [jmap-client](https://crates.io/crates/jmap-client) v0.4.0 | Client | Yes | No | Yes (tokio) | Production-ready | Apache/MIT |
| [libjmap](https://crates.io/crates/libjmap) v0.1.1 | Client | No | Yes (prototype) | Yes | "Will panic on many errors" | — |
| [melib](https://crates.io/crates/melib) | Multi-backend | Yes | No | Yes | GPLv3 | GPLv3 |
| [jmap](https://crates.io/crates/jmap) v0.0.5 | Types | Partial | Partial | No | Dead (2016) | — |
| [rusmes-jmap](https://crates.io/crates/rusmes-jmap) v0.1.0 | Server | Partial | No | Yes | Early | Apache |

**`jmap-client`** from Stalwart Labs is the only viable option. It covers RFC 8620 (Core), RFC 8621 (Mail), RFC 8887 (WebSocket). ~11K SLoC, builder pattern API, actively maintained. No contacts or calendars.

### JMAP Verdict

> **Update**: JMAP was implemented as a Rust provider using `jmap-client` 0.4. See `docs/jmap-rust-migration.md`.

**Not viable as a primary protocol for a multi-provider email client.** Only Fastmail supports it in production. Gmail, Outlook, Yahoo, iCloud — none do and none will. Adding JMAP would mean maintaining a third provider type for ~1-2% of users.

It could make sense as a future nice-to-have for Fastmail users (their JMAP is faster and more capable than their IMAP), but it is not a strategic priority. The implementation cost would be moderate given `jmap-client`'s quality and our existing `EmailProvider` abstraction — a `JmapProvider` behind the same interface.

---

## Non-Rust Alternatives (FFI)

| Library | Language | Assessment |
|---------|----------|-----------|
| **UW c-client** | C | Dead. Build system is painful, constant security vulnerabilities. PHP's `imap` extension still uses it. Not recommended. |
| **libcurl IMAP** | C | Not a full IMAP client. Cannot do IDLE, BODYSTRUCTURE parsing is broken for multi-line responses. Designed for simple fetch-one-message use cases. |
| **VMime** | C++ | Old codebase, small community. More capable than libcurl but not enough to justify FFI complexity. |
| **Thunderbird nsImapProtocol** | C++ | Battle-tested against every server on earth, but deeply intertwined with Mozilla's XPCOM. Not extractable. |
| **go-imap** | Go | The gold standard — v2 supports IMAP4rev2, mature ecosystem, powers multiple production clients. But Go FFI from Rust/Tauri is impractical. |

There is no good C/C++ IMAP client library suitable for FFI wrapping. The mature implementations are either dead, limited, or non-extractable. FFI would also add significant complexity to the Tauri build pipeline.

---

## Recommendations

### The reality

IMAP is a 23-year-old protocol where every server implements it differently. **Any production multi-provider email client will need server-specific workarounds regardless of which library it uses.** Thunderbird has decades of accumulated hacks for exactly this reason. Our raw TCP fallback is not a failure — it is the reality of IMAP.

The Rust IMAP ecosystem is fragmented and undermaintained compared to Go or even JavaScript. No crate supports IMAP4rev2. The two ecosystems (imap-proto-based and imap-codec-based) each have significant drawbacks.

### Short-term (now)

**Stay on async-imap.** It has the most real-world exposure (305K downloads), is actively maintained by the Delta Chat team, and works with Gmail/Outlook/Yahoo — the providers that matter. Expand the raw TCP fallback as needed for broken servers.

Specific improvements to make within the current architecture:
- Add PLAIN authentication support (fixes issue #204 — goneo.de and similar providers)
- Add a cancel mechanism for test connections
- Consider contributing patches to async-imap upstream for parser resilience

### Medium-term (6-12 months)

**Watch the imap-next/imap-codec ecosystem.** The architecture is genuinely better (quirk features, fuzz-testing, QRESYNC potential). But it needs to:
- Fix the Gmail SELECT crash (issue #641)
- Resolve io-imap's fundamental architectural questions (response routing, unsolicited responses)
- Ship a stable 1.0 release
- Gain more real-world usage beyond Himalaya

If these happen, migrating from async-imap to imap-client would let us delete `raw.rs` and gain QRESYNC for faster sync.

### Long-term considerations

> **Update**: Microsoft Graph provider was implemented in Rust. See `docs/graph-rust-migration.md`.

- **IMAP4rev2**: No Rust crate supports it. When Dovecot/Cyrus deployments start requiring it, the ecosystem will need to catch up. This may force a choice between contributing to an existing crate or building our own parser.
- **JMAP**: Only worth adding if Fastmail users become a meaningful segment. Low priority. **Update**: JMAP was subsequently implemented. See `docs/jmap-rust-migration.md`.
- **Microsoft Graph API**: If we want first-class Outlook support (beyond basic IMAP), the proprietary API is the practical path — same pattern as our Gmail API provider. **Update**: Microsoft Graph provider was implemented in Rust. See `docs/graph-rust-migration.md`.

### What we should NOT do

- **Do not migrate to imap-next/io-imap today.** It crashes on Gmail, has unresolved design flaws, and zero closed issues. The theoretical benefits (fuzz-testing, QRESYNC) do not outweigh the practical risks.
- **Do not attempt FFI with C/C++ IMAP libraries.** The viable options don't exist, and the build complexity isn't worth it.
- **Do not invest heavily in JMAP.** Provider adoption makes it a Fastmail-only feature for the foreseeable future. **Update**: JMAP was subsequently implemented for Fastmail support. See `docs/jmap-rust-migration.md`.
