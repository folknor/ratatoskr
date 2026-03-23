# Roaming Signatures

**Tier**: 2 — Keeps users from going back
**Status**: ⚠️ **Backend complete, UI missing** — DB schema with sync columns (`server_id`, `server_html_hash`, `source`, `last_synced_at`, `is_reply_default`, `body_text`). Gmail bidirectional sync via `sendAs` (pull server signatures on initial+delta sync, push local edits). JMAP Identity signature sync (`sync_jmap_identity_signatures`). Inline image extraction from signature HTML (base64 data-URI and CID parsing, dedup via xxh3, storage in inline image store). Exchange has no public API for signatures and never will (see Research §1–2). Missing: signature placement in compose UI.

---

- **What**: Signatures stored server-side, synced across clients

## Cross-provider behavior

| Provider | Native support | API |
|---|---|---|
| Exchange (Graph) | Roaming signatures (relatively new, ~2021) | Graph beta endpoints / EWS roaming settings |
| Gmail API | Signature in settings | `users.settings.sendAs` — per-alias signatures |
| JMAP | Nothing standardized | N/A |
| IMAP | Nothing | N/A |

## Pain points

- First-run experience: user adds their Exchange account, expects their signature to appear in compose automatically. If we don't fetch it, they have to manually recreate it — immediate negative impression.
- HTML signatures: signatures are rich HTML (logos, formatted text, links). Need to render them in compose and handle the boundary between user-typed content and the signature block.
- Multiple signatures: Exchange supports multiple signatures (new email vs reply). Gmail supports per-alias signatures. Need a signature picker or smart default (use reply signature for replies, new-email signature for new compose).
- JMAP/IMAP accounts: purely local signatures. Need a signature editor that stores locally. Same UI, just no server sync.
- Signature images: signatures often contain inline images (company logos, headshots). These are the 14KB PNGs that compound at volume. When fetching a roaming signature, need to extract inline images and deduplicate them in the attachment store.
- Corporate-managed signatures: some orgs push signatures via Exchange transport rules (appended server-side on send). Client-side signature would double up. Need to detect this — if the server appends a signature, don't insert one client-side. Hard to detect reliably.

## Work

- ✅ DB schema extended with sync columns (`server_id`, `server_html_hash`, `source`, `last_synced_at`, `is_reply_default`, `body_text`)
- ✅ Gmail `sendAs` signature fetch — pulled on initial sync and delta sync (`sync_signatures` in `crates/gmail/src/sync/labels.rs`)
- ✅ Gmail bidirectional sync — local edits pushed via `update_send_as_signature` (`crates/gmail/src/api.rs`), conflict resolution by server HTML hash
- ✅ JMAP Identity signature sync — `sync_jmap_identity_signatures` in `crates/jmap/src/signatures.rs`, upserts `htmlSignature`/`textSignature` keyed by `(account_id, server_id)`
- ✅ Inline image handling — `crates/provider-utils/src/signature_images.rs` extracts base64 data-URIs and CID references from signature HTML, deduplicates via xxh3, stores in inline image store
- ✗ Exchange — **permanently blocked.** No public API exists and Microsoft has explicitly confirmed there never will be (see Research §1–2). Sent-mail heuristic not worth the effort. Exchange users create their signature locally on first account add.
- ⬚ Signature placement in compose (iced UI work)

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for the iced (pure Rust) rewrite. All provider interactions are raw HTTP via `reqwest` (Graph, Gmail) or `jmap-client` (JMAP). The existing `signatures` table and `DbSignature` struct handle local CRUD; this research covers what's needed for server-side sync and first-run population.

---

### 1. Exchange Roaming Signatures via Microsoft Graph

#### Current state: No API exists, and Microsoft says there never will be

As of March 2026, **Microsoft Graph has no endpoint for reading or writing roaming signatures** — not in v1.0, not in beta. The `GET /me/mailboxSettings` endpoint returns `automaticRepliesSetting`, `language`, `timeZone`, `dateFormat`, `timeFormat`, `delegateMeetMessageDeliveryOptions`, and `userPurpose`. Signatures are explicitly absent. The beta `mailboxSettings` schema is identical — no signature properties.

**Microsoft has explicitly stated they have no plans to add this.** From the [Graph API Support for Roaming Signatures feature request](https://techcommunity.microsoft.com/idea/microsoft365developerplatform/graph-api-support-for-roaming-signatures/4106799) (April 2024): "We have _no plans to support roaming signature management_ in the Microsoft Graph API." Microsoft's recommended alternative is Outlook Add-ins with event-based hooks (`OnMessageCompose`), which is irrelevant to third-party email clients.

This is despite Microsoft having broken EWS signature access when they rolled out roaming signatures (April 2023) and promising a Graph replacement that was never delivered.

#### Where roaming signatures actually live

Roaming signatures are stored in an internal "Substrate" store (moved from the mailbox FAI in April 2023), not accessible through any documented Graph, EWS, or PowerShell endpoint. Outlook clients read from this location directly using internal protocols.

#### All known approaches — exhaustively checked

| Approach | Status | Detail |
|----------|--------|--------|
| Graph `mailboxSettings` | **No signature property** | Checked v1.0 and beta schemas. Not present. |
| Graph beta endpoints | **Nothing** | No signature endpoint in any beta changelog through March 2026. |
| EWS `OWA.UserOptions` FAI | **Broken since April 2023** | Roaming signatures moved storage; FAI contains stale data. |
| PowerShell `Get-MailboxMessageConfiguration` | **Broken when roaming enabled** | `SignatureHtml`/`SignatureText` params don't work with roaming signatures (default for all tenants since 2023). Only works if admin disables roaming via support ticket. |
| Outlook Add-ins (`OnMessageCompose`) | **Irrelevant** | Only works inside Outlook. We're a standalone client. |
| Sent-mail heuristic | **Fragile, one-time** | Parse recent sent emails, extract signature by pattern matching. Best-effort first-run population. |

#### Architecture implication

**Exchange signatures cannot be fetched via any public API, and Microsoft has confirmed this will not change.** The only option is:

1. **Do nothing** — user manually creates their signature locally. On first launch, show "Set up your signature" prompt. This is the pragmatic choice.
2. **Sent-mail heuristic** — fetch recent sent emails, extract signature by pattern matching common markers (`-- `, `<div class="Signature">`, etc.). Fragile, locale-dependent, and not worth the implementation effort given it only saves the user one copy-paste.

**Recommendation**: Option 1. Prompt the user to set up their signature on first Exchange account add. Don't waste time on the sent-mail heuristic.

---

### 2. Exchange Signatures via EWS (Legacy)

#### The old approach: `OWA.UserOptions`

Before roaming signatures, OWA stored the user's signature in a FAI (folder-associated item) named `OWA.UserOptions` in the Inbox. EWS could access this via `UserConfiguration.Bind()` with dictionary keys: `signaturehtml`, `signaturetext`, `autoaddsignature`, `autoaddsignatureonreply`, `signaturedefault`.

#### This no longer works

Microsoft enabled roaming signatures across all Office 365 tenants in April 2023. When roaming signatures are enabled, OWA **ignores** the `OWA.UserOptions` configuration. The FAI may still exist with stale data, but it is no longer the source of truth. Signature storage was moved to an internal "Substrate" store accessible only to Outlook's proprietary protocol.

Additionally, **EWS itself is being retired in October 2026**, making any EWS-based approach doubly dead.

**Bottom line**: EWS signature access is dead. Do not rely on it.

---

### 3. Gmail `users.settings.sendAs`

#### API endpoint

```
GET https://gmail.googleapis.com/gmail/v1/users/me/settings/sendAs
```

Returns all send-as aliases. Each alias is a `SendAs` resource with:

| Field | Type | Notes |
|-------|------|-------|
| `sendAsEmail` | string | The "From" address |
| `displayName` | string | Display name for this alias |
| `signature` | string | HTML signature body (max 10,000 chars including markup) |
| `isPrimary` | bool | Whether this is the account's primary address |
| `isDefault` | bool | Whether this is the default "From" address |

#### Signature field details

- **Format**: HTML string. Gmail sanitizes on write (strips dangerous elements).
- **Scope**: Per-alias. Each send-as alias has its own independent signature.
- **Application**: Gmail web UI appends the signature to new compose only. Replies/forwards do not auto-append.

#### Writing signatures

```
PUT /gmail/v1/users/me/settings/sendAs/{sendAsEmail}
```

With full `SendAs` resource in the body.

#### Existing codebase support

The `GmailSendAs` struct in `crates/gmail/src/types.rs` already deserializes the signature field. The `send_as_aliases` table has a `signature_id` FK pointing to the `signatures` table. On first sync, extract the `signature` HTML from the Gmail API response and insert it into the local `signatures` table.

#### Required OAuth scope

`https://www.googleapis.com/auth/gmail.settings.basic` — covers read/write access to `sendAs` settings including signatures.

---

### 4. JMAP Identity Signatures

#### RFC 8621 Identity object

RFC 8621 Section 6 defines the `Identity` type under the `urn:ietf:params:jmap:submission` capability:

| Field | Type | Server-set? | Description |
|-------|------|-------------|-------------|
| `id` | `Id` | Yes | Immutable identifier |
| `name` | `String` | No | Display name for "From" |
| `email` | `String` | Yes | Email address (immutable) |
| `textSignature` | `String` | No | Plain-text signature |
| `htmlSignature` | `String` | No | HTML signature |

Both `textSignature` and `htmlSignature` are **client-settable** via `Identity/set`. This makes JMAP the cleanest provider for signature sync.

#### `jmap-client` crate support

The `jmap-client` crate (v0.4, Stalwart Labs) fully supports Identity signatures:

```rust
// Reading
let html_sig: Option<&str> = identity.html_signature();
let text_sig: Option<&str> = identity.text_signature();

// Writing
identity.html_signature("<p>My signature</p>");
identity.text_signature("My signature");
```

For JMAP accounts, signatures round-trip through the server. On account setup, fetch `Identity/get`, extract `htmlSignature` into local `signatures` table. On local edit, write back via `Identity/set`.

---

### 5. Signature HTML Format

#### Outlook signature HTML

Outlook-generated signatures use Word's HTML engine, producing verbose, deeply nested HTML with `mso-*` CSS properties, `MsoNormal` classes, dimensions in mixed units (points, inches, pixels), and tables for multi-column layouts.

#### Gmail signature HTML

Gmail sanitizes aggressively. Typical output is clean `<div dir="ltr">` with basic formatting tags.

#### Inline images: CID vs base64 vs linked

| Method | Used by | Notes |
|--------|---------|-------|
| CID references | Outlook | `<img src="cid:uuid@domain">`, image attached as MIME part |
| Base64 data URIs | Some generators | Outlook **blocks rendering**. Gmail strips them. |
| Linked (HTTP URLs) | Gmail, web editors | Requires internet. May be tracked. |

#### Architecture implication for signature import

When importing signatures: parse HTML for `<img src="cid:...">` references, resolve CID images from MIME structure, store in inline image store, rewrite references to local paths. Base64 data URIs can be decoded and stored directly.

---

### 6. Corporate Transport Rule Signatures

#### The duplication problem

If the org appends a signature via Exchange transport rule **and** the user has a client-side signature, the message gets two signatures.

#### Detection strategies

There is **no reliable programmatic way** to detect transport-rule signatures. The pragmatic approach:

**Recommendation**: Provide a clear setting: "My organization adds signatures automatically" (default: off). When enabled, suppress client-side signature insertion. Optionally offer a "send a test email to yourself" flow.

---

### 7. Data Model

#### Current schema

The existing `signatures` table has: `id`, `account_id`, `name`, `body_html`, `is_default`, `sort_order`, `created_at`.

#### Changes needed for server sync

```sql
ALTER TABLE signatures ADD COLUMN server_id TEXT;
ALTER TABLE signatures ADD COLUMN body_text TEXT;
ALTER TABLE signatures ADD COLUMN is_reply_default INTEGER DEFAULT 0;
ALTER TABLE signatures ADD COLUMN source TEXT DEFAULT 'local';
  -- 'local' | 'gmail_sync' | 'jmap_sync' | 'exchange_parsed'
ALTER TABLE signatures ADD COLUMN last_synced_at INTEGER;
ALTER TABLE signatures ADD COLUMN server_html_hash TEXT;
CREATE UNIQUE INDEX idx_signatures_server ON signatures(account_id, server_id)
    WHERE server_id IS NOT NULL;
```

#### Conflict resolution

For Gmail and JMAP (providers with read-write APIs):
- On sync, compute hash of server HTML. Compare with stored `server_html_hash`.
- If server changed and local didn't: update local.
- If local changed and server didn't: push to server.
- If both changed: prefer server (safest for enterprise).

---

### 8. Signature Placement in Compose

#### The `-- \n` separator

RFC 3676 Section 4.3 defines the signature separator: `-- ` (dash dash space, followed by newline). In HTML, conventions vary: Gmail uses `<div class="gmail_signature">`, Outlook uses `<div id="Signature">`, Thunderbird inserts `-- <br>` literally.

#### Placement strategy

| Scenario | Signature position |
|----------|-------------------|
| New compose | Bottom of body, after a blank line |
| Reply (top-posting) | Between new content and quoted text |
| Forward | Same as reply top-posting |

Wrap the signature in `<div id="ratatoskr-signature" data-signature-id="{uuid}">` for replacement, stripping on reply, and edit boundary handling.

#### `is_default` vs `is_reply_default`

Exchange supports two defaults: one for new compose, one for replies/forwards. Gmail does not make this distinction. Use `is_default` for new compose, `is_reply_default` for reply/forward. If only `is_default` set, use it for all types.

---

### 9. Provider Capability Summary

| Capability | Exchange (Graph) | Gmail | JMAP | IMAP |
|-----------|-----------------|-------|------|------|
| Read signatures from server | **No API** | Yes (`sendAs.signature`) | Yes (`Identity.htmlSignature`) | No |
| Write signatures to server | **No API** | Yes (`sendAs.update`) | Yes (`Identity/set`) | No |
| Per-alias signatures | N/A | Yes (per send-as alias) | Yes (per Identity) | No |
| New vs reply defaults | N/A (no read) | No (new only) | No (spec doesn't distinguish) | No |
| First-run auto-populate | Sent-mail heuristic only | API fetch | API fetch | No |
| Bidirectional sync | No | Yes | Yes | No |

#### Implementation priority

1. **Gmail fetch on account setup** — highest value, easiest implementation. `GmailSendAs` already has the `signature` field.
2. **JMAP fetch/push on account setup** — clean API via `jmap-client`. Bidirectional.
3. **Local signature editor** — already exists. Needs `body_text` field and sync columns.
4. **Sent-mail heuristic for Exchange** — medium effort, fragile. Defer to post-MVP.
5. **Transport rule detection** — user setting, not auto-detection. Low effort.
