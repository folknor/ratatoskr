# Microsoft Graph → Rust Migration Plan

**Date**: March 2026
**Status**: Deferred (blocked on JMAP completion + trait extraction)
**Goal**: Implement Microsoft Graph Mail API as a Rust-native email provider. This is step 3 in the execution order from `docs/rust-provider-crate-research.md`.

This document captures known decisions, open questions, and deferred items so they aren't lost. It will be expanded into a full plan after the JMAP migration is complete and the shared `EmailProvider` trait has been extracted.

---

## Table of Contents

- [Why Graph Third](#why-graph-third)
- [Known Decisions](#known-decisions)
- [Open Questions](#open-questions)
- [Key Differences from Gmail and JMAP](#key-differences-from-gmail-and-jmap)
- [Deferred Items](#deferred-items)

---

## Why Graph Third

1. **Depends on trait extraction** — Graph is the first provider that should be built AGAINST the shared `EmailProvider` trait extracted from Gmail + JMAP. Building it before that trait exists would mean another one-off implementation to refactor.
2. **OAuth infrastructure must be multi-provider first** — Graph requires Microsoft OAuth2 (Entra ID, `/common/oauth2/v2.0/` endpoints). The existing `oauth.rs` is Google-specific. It needs to be generalized for at least two providers before Graph can use it. This generalization should happen naturally during Gmail Rust migration, but must be verified.
3. **Folder-centric model is the hardest to reconcile** — Gmail is label-centric (messages have multiple labels). JMAP uses mailboxes (messages can belong to multiple mailboxes). Graph is folder-centric (messages live in exactly one folder). This is the most restrictive model and the hardest to map onto our Gmail-style label UI. Seeing how the trait handles Gmail labels vs JMAP mailboxes will inform how Graph folders fit.
4. **Lower priority user base** — Outlook.com/Exchange users can already connect via IMAP+OAuth2 (see quick win below). Graph adds richer features (categories, delta sync, focused inbox) but is not a blocker for basic access.

---

## Known Decisions

These carry forward from `docs/rust-provider-crate-research.md` and `docs/microsoft-exchange-assessment.md`:

### 1. Hand-roll on reqwest, no `graph-rs-sdk`

~18 REST endpoints. `graph-rs-sdk` covers the entire Graph API surface (not just Mail), has single-maintainer risk (last commit Aug 2025), and brings its own OAuth layer. Not worth the dependency. Same rationale as Gmail's rejection of `google-gmail1`.

### 2. Microsoft Graph API, not EWS

EWS is deprecated for Exchange Online (Oct 2026 block, Apr 2027 permanent removal). Graph works with both Exchange Online and personal Outlook.com/Hotmail accounts. REST/JSON vs SOAP/XML. No contest.

### 3. OAuth2 via Entra ID (formerly Azure AD)

- Authorization Code + PKCE flow, same pattern as Gmail
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
- Auth endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/authorize`
- Scopes: `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`, `offline_access`
- Localhost redirect (port 17248-17251, same server as Gmail)
- Multi-tenant + personal accounts (use `/common` tenant)

### 4. Commands will be `graph_*` prefixed

Same pattern as `gmail_*` and `jmap_*`. Unless the shared trait and provider-agnostic commands are ready by the time Graph ships — in which case Graph may be the first provider to use them directly.

### 5. Delta sync per folder, not global

Graph's delta endpoint is per-folder: `GET /me/mailFolders/{id}/messages/delta`. Returns `@odata.deltaLink` for next sync. Delta tokens don't expire (unlike Gmail's ~30-day History API window). Must track delta tokens per folder in DB — similar to IMAP's per-folder UIDVALIDITY tracking.

### 6. On-premises Exchange is out of scope

On-prem Exchange supports IMAP — users can connect via our existing IMAP provider. EWS for on-prem is niche and the SOAP/XML complexity isn't justified. If demand emerges, revisit later with `ews-rs` types from Thunderbird.

---

## Open Questions

These must be resolved before writing the full plan:

### 1. App registration model

Gmail uses user-provided Client IDs (configured in Settings). Microsoft Graph requires an Azure AD app registration. Options:
- **Ship a default app registration** — simpler for users, but we'd need to manage it (including publisher verification for organizational accounts).
- **User provides their own** — same as Gmail, but Azure portal is more complex than Google Cloud Console.
- **Both** — ship a default for personal accounts, allow override for organizational.

### 2. Folder-centric to label-centric mapping

Graph messages live in exactly one folder. Our UI is label-centric (threads can have multiple labels). Options:
- **Folder = primary label** — treat the message's folder as its only "label." Sidebar shows folders, not labels. Simplest, but loses multi-label UX for Graph accounts.
- **Categories as supplementary labels** — Graph supports color-coded categories on messages (similar to Gmail labels). Map Graph categories → labels in our UI. Folders determine location, categories add metadata.
- **Hybrid** — folder membership is the base, categories provide additional labels. This is how Outlook itself works.

This is a product decision, not just an adapter detail.

### 3. Thread model

Graph has a `conversationId` field that groups related messages, but it's not as reliable as Gmail's threading. Graph also has `conversationIndex` (binary threading data from Exchange). Options:
- Use `conversationId` as `threadId` (simplest, may produce different groupings than users expect).
- Use our JWZ threading algorithm on `Message-ID`/`References`/`In-Reply-To` headers (more accurate, more work).
- Use `conversationId` as primary, fall back to JWZ for edge cases.

### 4. Rate limit handling

Graph allows only **4 concurrent requests per app per mailbox**. This is more restrictive than Gmail. The sync engine's concurrency model must respect this — probably means serial or very low concurrency (2-3) for delta sync fetches.

### 5. Shared trait readiness

By the time Graph starts, will the `EmailProvider` trait be extracted from Gmail + JMAP? If yes, Graph is the first provider built against it. If no, Graph is another one-off and we have three providers to reconcile later.

---

## Key Differences from Gmail and JMAP

| Aspect | Gmail | JMAP | Graph |
|--------|-------|------|-------|
| **Membership model** | Labels (multi-label) | Mailboxes (multi-mailbox) | Folders (single folder) + Categories |
| **Threading** | Native `threadId` | Native `threadId` | `conversationId` (less reliable) |
| **Delta sync** | Global History API (expires ~30 days) | Global `Email/changes` state strings | Per-folder delta tokens (don't expire) |
| **Concurrency** | Generous (10+ parallel) | Server-side batching | **4 concurrent max** |
| **Auth** | Google OAuth2 + PKCE | Basic or Bearer | Microsoft OAuth2 (Entra ID) + PKCE |
| **Send** | POST raw RFC 822 base64url | Email/import + EmailSubmission/set | POST `/me/sendMail` (JSON body, not raw) |
| **Attachments** | Part of message payload | Blob download by ID | Separate `/attachments/{id}` endpoint, upload sessions for >3MB |
| **Sync state storage** | `history_id` on account | `jmap_sync_state` table (per type) | Per-folder delta tokens (new table needed) |

### Graph-specific concerns

- **Send format**: Graph's `/me/sendMail` accepts a JSON message object, NOT raw RFC 822. We can't reuse `mail-builder` output directly for sending. Either: (a) build the JSON message body in Rust, or (b) use the MIME send endpoint (`/me/sendMail` with `Content-Type: text/plain` and raw MIME — undocumented but works). Need to investigate.
- **Large attachments**: Files >3MB require upload sessions (`/me/messages/{id}/attachments/createUploadSession`). This is a multi-step process unlike Gmail/JMAP where attachments are part of the message payload.
- **OData pagination**: All list endpoints use `@odata.nextLink` / `@odata.deltaLink`. Need a generic `ODataCollection<T>` wrapper struct with `#[serde(rename = "@odata.nextLink")]`.
- **Focused Inbox**: Graph exposes `inferenceClassification` (Focused/Other). Could map to our category system. Optional enrichment.

---

## Deferred Items

Items explicitly out of scope until Graph planning begins in earnest:

### From the JMAP migration

1. **Shared `EmailProvider` trait** — extract from Gmail + JMAP after JMAP Phase 1 is complete. This is the prerequisite for Graph.
2. **Provider-agnostic Tauri commands** — depends on the trait. Graph may be the first consumer.

### Graph-specific

3. **Microsoft OAuth2 in `oauth.rs`** — extend the existing Google-only OAuth server to handle Microsoft endpoints and scopes. The flow is identical (PKCE + localhost redirect), only endpoints and scopes differ. May happen during Gmail Rust migration if `oauth.rs` is generalized.
4. **Per-folder delta token storage** — new DB table for tracking delta tokens per folder per account. Schema TBD.
5. **Graph-to-label mapping strategy** — product decision on folders + categories → labels. Needs design review with UI implications.
6. **Thread ID strategy** — `conversationId` vs JWZ threading. Needs investigation of `conversationId` reliability across real accounts.
7. **Send format investigation** — JSON message body vs raw MIME. Test both paths, pick the one that handles attachments and encoding correctly.
8. **Large attachment upload sessions** — multi-step upload for >3MB files. Not critical for initial implementation (can limit to inline/small attachments), but needed for full parity.
9. **Webhook subscriptions** — Graph supports push notifications via webhooks for real-time sync. Requires a reachable endpoint (problem for desktop apps). Polling via delta sync is the initial approach. Investigate if Tauri can expose a local webhook receiver via the existing localhost server.
10. **Azure AD app registration** — create and configure the app registration. Publisher verification for organizational access. Decide on default-shipped vs user-provided model.
11. **Focused Inbox integration** — map Graph's `inferenceClassification` to our category tabs (Primary/Other mapping). Optional enrichment after basic sync works.
12. **Exchange on-premises via EWS** — only if significant demand. `ews-rs` from Thunderbird provides types, but no client. SOAP/XML complexity is high. On-prem users can use IMAP.

### Quick win (can happen before full Graph)

13. **IMAP + OAuth2 for Outlook.com** — add Microsoft OAuth2 flow, use XOAUTH2 SASL with our existing IMAP provider. Gives Outlook users immediate access without building the full Graph provider. Requires only: OAuth2 endpoint configuration in `oauth.rs`, Azure AD app registration, IMAP AUTHENTICATE with OAuth token (already supported in `connection.rs`). This is independent of the Graph migration and could ship at any time.

---

## References

- `docs/microsoft-exchange-assessment.md` — full ecosystem assessment (EWS vs Graph, crate evaluation, auth details, rate limits)
- `docs/rust-provider-crate-research.md` — crate decisions and strategic plan (Graph endpoints table, architecture decisions)
- `docs/gmail-rust-migration.md` — Gmail patterns that Graph will follow (token management, reqwest setup, sync-with-DB-writes)
- `docs/jmap-rust-migration.md` — JMAP patterns, trait extraction trigger
