# Reactions

**Tier**: ~~2 — Keeps users from going back~~ → **Dropped as a user-facing feature.**
**Status**: ❌ **No UI will be built.** Backend defensive code remains (Phases 1–4) to avoid breaking on reaction data during sync. Exchange is read-only with no write API (permanently blocked). Gmail has full read/write but via a completely different mechanism (MIME). JMAP/IMAP have nothing. There is no unified model — reactions cannot be presented as a consistent cross-provider feature. Unlike labels, there is no useful local-only fallback (reactions are social).

---

- **What**: Emoji reactions on email messages (Exchange/new Outlook feature)

## Cross-provider behavior

| Provider | Native support | Read | Write |
|---|---|---|---|
| Exchange (Graph) | Undocumented extended properties (no `reactions` property on message resource) | Yes — via `singleValueExtendedProperties` | **No** — no public API, confirmed March 2026 |
| Gmail API | MIME-based reactions (April 2025, default for all users Feb 2026) | Yes — detect `text/vnd.google.email-reaction+json` MIME part | Yes — send reaction MIME email |
| JMAP | Nothing — no RFC or extension | Can parse Gmail-originated reaction MIME | No |
| IMAP | Nothing | Can parse Gmail-originated reaction MIME | No |

## Pain points

- Phase 1 priority: even before displaying reactions, must not break when a message has reaction metadata. Defensive deserialization — ignore unknown fields rather than erroring.
- Display: reactions appear as a row of emoji chips below the message (like Slack/Teams). Each chip shows the emoji + count + who reacted. This is a new UI element with no existing equivalent in the client.
- Local-only reactions for non-Exchange: could implement local-only reactions that only the user sees. Questionable value — reactions are social, local-only defeats the purpose. Probably better to just not show the reaction UI on non-Exchange accounts.
- Sync: reactions can change after initial sync (someone reacts later). Need to handle updates to the reactions collection during delta sync.
- Adding reactions: **Gmail only.** There is no public Graph API to add email reactions — the `chatMessage:setReaction` endpoint is Teams-only. PATCHing `OwnerReactionType` as an extended property would only set a local flag on the user's own copy without triggering Exchange's server-side propagation to other recipients. For Gmail, reactions are sent as regular MIME emails.

## Work

No further work planned. Backend defensive code (Phases 1–4) stays to prevent sync breakage. No UI will be built — the feature cannot be unified across providers.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for the iced (pure Rust) rewrite. Reactions are primarily an Exchange feature, but Gmail added MIME-based reactions in 2025.

---

### 1. Exchange Graph API: Undocumented Extended Properties

The Graph API `message` resource (v1.0 and beta) has **no `reactions` property**. The `chatMessageReaction` resource type in Graph docs is for Teams chat messages only, not email. Email reactions are stored as **undocumented MAPI extended properties** under the GUID `{41F28F13-83F4-4114-A584-EEDB5A6B0BFF}`:

| Property name | Type | Description |
|---|---|---|
| `ReactionsCount` | Integer | Total number of reactions on the message |
| `ReactionsSummary` | Binary | Current state of reactions — reactor names, types, timestamps. Compressed binary format. **Most reliable property** per Glen Scales' testing. |
| `OwnerReactionType` | String | The mailbox owner's own reaction (if any) |
| `OwnerReactionTime` | SystemTime | When the mailbox owner reacted |
| `MapiReactionsBlob` | Binary | JSON document with all reactions. Deprecated — exhibits client-dependent behavior. |
| `ReactionsHistory` | Binary | Alternative to MapiReactionsBlob. Also client-dependent; not reliably available. |

**Reading reactions via Graph**: Request extended properties with `$expand` or `$filter`:

```
GET /me/messages?$filter=singleValueExtendedProperties/any(
  ep: ep/id eq 'Integer {41F28F13-83F4-4114-A584-EEDB5A6B0BFF} name ReactionsCount'
  and cast(ep/value, Edm.Int32) gt 0
)
```

The `ReactionsSummary` is a binary blob with unpublished format. The only known parser is in the MSGReader .NET library (ported to PowerShell by Glen Scales). The format uses simple serialization (not compression) and contains reactor identities, reaction types, and timestamps. `ReactionsSummary` is more reliable than `MapiReactionsBlob` or `ReactionsHistory`, which exhibit client-dependent behavior and may be deprecated.

**Writing reactions**: There is **no public Graph API endpoint** to add or remove an email reaction, confirmed as of March 2026. The `chatMessage:setReaction` endpoint is **Teams chat only** — it does not apply to email messages. PATCHing `OwnerReactionType` via `singleValueExtendedProperties` would only update the local flag on the authenticated user's own copy of the message — it does **not** trigger Exchange's server-side reaction propagation to other recipients' mailboxes. The propagation mechanism is an internal Exchange behavior triggered by Outlook's proprietary protocol, not by writing MAPI properties. Glen Scales (Exchange developer) confirms: "The lack of a public API for Outlook Reactions remains a frustrating limitation."

---

### 2. Outlook's Reaction Model

Outlook supports exactly **six reaction types**: like (thumbs up), love (heart), celebrate, laugh, surprised, sad. The set is **not customizable** — it is a fixed enum mapped to named strings internally.

Key behaviors:
- Reactions are same-tenant by default but work cross-tenant between Exchange Online orgs
- Do **not** work for shared mailboxes, GCC High, DoD, or Gallatin environments
- When sent to non-Exchange recipients, Outlook falls back to sending a **regular email notification** (see §2.1 below)
- The `x-ms-reactions: disallow` header suppresses the reaction UI
- BCC recipients can react, visible only to themselves and the sender
- Reactions do **not** appear to modify `lastModifiedDateTime` on the message — delta sync implications (see section 6)

#### 2.1 Cross-Boundary Fallback Notifications

When an Exchange user reacts to a message from a non-Exchange sender, Exchange sends a **fallback notification email** to the original sender. Two notification mechanisms exist:

1. **Reaction Daily Digest** — sent to the *original message author* (who must be an Exchange user) summarizing reactions their messages received over the past 24 hours. Sent from `no-reply@outlook.mail.microsoft` with subject `Reaction Daily Digest - [Day], [Month] [Date], [Year]`. HTML-formatted with reaction emojis rendered as remote images. Users can unsubscribe via `outlook.office365.com/owa/ReactionDigestMailUnsubscribe.aspx`. This is an intra-Exchange notification — it tells an Exchange sender about reactions, it doesn't cross provider boundaries.

2. **Cross-boundary fallback email** — sent when the reactor is on Exchange but the original sender is external (e.g., Gmail, IMAP). Microsoft's docs confirm this exists ("the reaction will be sent in the form of a fallback email instead") but provide **no documentation** on its format:
   - No known subject line pattern
   - No custom headers (no `x-ms-reaction` or equivalent marker)
   - No machine-readable MIME part (unlike Gmail's `text/vnd.google.email-reaction+json`)
   - Likely a plain notification email ("X reacted to your message") with no structured metadata
   - Locale-dependent — subject/body presumably varies by language

**Can we detect these?** Not reliably. Unlike Gmail reactions which have a dedicated content type (`text/vnd.google.email-reaction+json`), Exchange's fallback emails are opaque notification messages with no documented machine-readable markers. Heuristic detection (subject pattern matching, sender matching) would be fragile and locale-dependent. **Not worth pursuing.**

---

### 3. Gmail Reactions: MIME-Based

Gmail launched emoji reactions in April 2025, enabled by default for all users as of February 2026. Unlike Exchange, Gmail implements reactions as **separate MIME email messages**:

```
Content-Type: multipart/alternative; boundary="boundary"

--boundary
Content-Type: text/plain; charset="UTF-8"
[emoji] Sender reacted to your message

--boundary
Content-Type: text/vnd.google.email-reaction+json; charset="UTF-8"
{"emoji":"👍","version":1}

--boundary
Content-Type: text/html; charset="UTF-8"
<html>...</html>
--boundary--
```

Key details:
- The reaction email has `In-Reply-To` pointing to the original message's `Message-ID`
- JSON payload: `emoji` (one Unicode emoji) and `version` (must be `1`)
- Maximum 20 distinct recipients per reaction, 20 reactions per user per message
- Disabled for mailing lists

**API access**: No dedicated "reactions" endpoint. Reactions are regular emails visible in `messages.list`/`messages.get`. Detect via `text/vnd.google.email-reaction+json` content type. To **send** a reaction, compose and send a regular email with the reaction MIME part.

---

### 4. JMAP / IMAP Reactions

No JMAP RFC or extension exists for reactions. No IMAP mechanism exists. Gmail-originated reactions arrive as regular messages with the reaction MIME part, which can be parsed.

---

### 5. Emoji Rendering in iced

Iced uses `cosmic-text` for text shaping and `swash` for rasterization. Color emoji support has known gaps — COLRv1, CBDT/CBLC bitmap, and SVG-based emoji fonts are not supported by swash.

**Practical path**: Since Outlook uses a fixed set of six named types, use **pre-rendered emoji images or SVG assets** mapped to the six reaction names. For Gmail reactions (arbitrary Unicode emoji), use `iced::widget::image` with pre-rasterized emoji PNGs from Twemoji or Noto Emoji, mapping each Unicode codepoint to its image file. Fallback to monochrome Noto Emoji for unsupported codepoints.

---

### 6. Delta Sync Considerations

**Exchange**: Reactions live in extended MAPI properties, not on the core message resource. A new reaction does **not** appear to update `lastModifiedDateTime` or `changeKey`, meaning **delta queries will not surface reaction changes**. Must periodically re-fetch `ReactionsCount` for messages in view, or accept stale counts until next full sync.

**Gmail**: Reactions are separate messages, so they appear naturally in `history.list` as new message additions. No special handling needed beyond MIME detection.

---

### 7. Data Model

```sql
CREATE TABLE message_reactions (
    message_id   TEXT NOT NULL,
    account_id   TEXT NOT NULL,
    reactor_email TEXT,
    reactor_name  TEXT,
    reaction_type TEXT NOT NULL,  -- "like", "heart", etc. for Exchange; emoji codepoint for Gmail
    reacted_at   TEXT,            -- ISO 8601, nullable
    source       TEXT NOT NULL,   -- "exchange_native" | "gmail_mime" | "imap_mime"
    PRIMARY KEY (message_id, account_id, reactor_email, reaction_type)
);

CREATE INDEX idx_reactions_message ON message_reactions(message_id, account_id);
```

The `source` column lets the UI know whether to render from the fixed Outlook set or as arbitrary emoji.

---

### 8. Defensive Deserialization

Serde's default behavior is to **silently ignore unknown fields** — exactly what we want. Key rules:
- **Do not** use `#[serde(deny_unknown_fields)]` on any struct deserializing Graph API responses
- Define structs with only the fields we care about, rely on serde's default ignore-unknown behavior
- For `ReactionsSummary` binary: deserialize as `Option<String>` (base64-encoded), parse separately. If parsing fails, log a warning and treat as zero reactions. Never panic.

---

### 9. Implementation Priority

1. ✅ **Phase 1 (defensive)**: Serde default ignore-unknown-fields handles reaction fields. Gmail `is_reaction` flag on messages prevents reaction MIME from rendering as body text. Reaction-only messages excluded from thread aggregates in `sync/src/persistence.rs`.
2. ✅ **Phase 2 (Gmail read)**: `extract_reaction_emoji()` in `gmail/src/parse.rs` parses `text/vnd.google.email-reaction+json` MIME parts during sync. `insert_reactions()` in `gmail/src/sync/storage.rs` resolves target via `In-Reply-To` and populates `message_reactions` with `source = 'gmail_mime'`. Migration v37 in `db` crate.
3. ✅ **Phase 3 (Exchange read)**: `extract_reaction_properties()` in `graph/src/parse.rs` reads `OwnerReactionType` and `ReactionsCount` extended properties (GUID `{41F28F13-...}`). `insert_exchange_reactions()` in `graph/src/sync/persistence.rs` stores owner reaction + count metadata. `refresh_reactions_for_recent_messages()` in the same module polls via `$batch` every 5th sync cycle to catch reaction changes missed by delta queries.
4. ✅ **Phase 4 (Gmail write)**: `send_reaction()` in `gmail/src/ops.rs` builds correct MIME structure with `build_reaction_mime()` and sends via Gmail API.
5. **Phase 5 (Exchange write)**: **Permanently blocked.** No public Graph API exists for adding email reactions (confirmed March 2026). The `chatMessage:setReaction` endpoint is Teams-only. PATCHing `OwnerReactionType` as an extended property only updates the local copy — no server-side propagation to other recipients. The reaction propagation mechanism is internal to Exchange/Outlook's proprietary protocol. Do not reverse-engineer. If Microsoft ever ships a public API, revisit.
