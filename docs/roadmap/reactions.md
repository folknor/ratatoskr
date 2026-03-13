# Reactions

**Tier**: 2 — Keeps users from going back
**Status**: ❌ **Not implemented**

---

- **What**: Emoji reactions on email messages (Exchange/new Outlook feature)

## Cross-provider behavior

| Provider | Native support |
|---|---|
| Exchange (Graph) | Full — `reactions` collection on message |
| Gmail API | Nothing |
| JMAP | Nothing |
| IMAP | Nothing |

## Pain points

- Phase 1 priority: even before displaying reactions, must not break when a message has reaction metadata. Defensive deserialization — ignore unknown fields rather than erroring.
- Display: reactions appear as a row of emoji chips below the message (like Slack/Teams). Each chip shows the emoji + count + who reacted. This is a new UI element with no existing equivalent in the client.
- Local-only reactions for non-Exchange: could implement local-only reactions that only the user sees. Questionable value — reactions are social, local-only defeats the purpose. Probably better to just not show the reaction UI on non-Exchange accounts.
- Sync: reactions can change after initial sync (someone reacts later). Need to handle updates to the reactions collection during delta sync.
- Compose: adding a reaction is a PATCH to the message on Graph. Need to handle the case where the user reacts to a message but is offline (queue and sync later? or require connectivity?).

## Work

Phase 1 — defensive deserialization. Phase 2 — display reactions on Exchange messages. Phase 3 — allow reacting on Exchange accounts. Skip local fallback.

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
| `ReactionsSummary` | Binary | Serialized blob: reactor identity, reaction type, timestamp per reaction |
| `OwnerReactionType` | String | The mailbox owner's own reaction (if any) |
| `OwnerReactionTime` | SystemTime | When the mailbox owner reacted |

**Reading reactions via Graph**: Request extended properties with `$expand` or `$filter`:

```
GET /me/messages?$filter=singleValueExtendedProperties/any(
  ep: ep/id eq 'Integer {41F28F13-83F4-4114-A584-EEDB5A6B0BFF} name ReactionsCount'
  and cast(ep/value, Edm.Int32) gt 0
)
```

The `ReactionsSummary` is a binary blob with unpublished format. The only known parser is in the MSGReader .NET library.

**Writing reactions**: There is **no documented Graph API endpoint** to add or remove an email reaction. Ratatoskr can display reactions but cannot programmatically add them through any public API.

---

### 2. Outlook's Reaction Model

Outlook supports exactly **six reaction types**: like (thumbs up), love (heart), celebrate, laugh, surprised, sad. The set is **not customizable** — it is a fixed enum mapped to named strings internally.

Key behaviors:
- Reactions are same-tenant by default but work cross-tenant between Exchange Online orgs
- Do **not** work for shared mailboxes, GCC High, DoD, or Gallatin environments
- When sent to non-Exchange recipients, Outlook falls back to sending a **regular email notification**
- The `x-ms-reactions: disallow` header suppresses the reaction UI
- BCC recipients can react, visible only to themselves and the sender
- Reactions do **not** appear to modify `lastModifiedDateTime` on the message — delta sync implications (see section 6)

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

1. **Phase 1 (defensive)**: Ensure all Graph/Gmail deserialization tolerates reaction fields. For Gmail, detect and skip `text/vnd.google.email-reaction+json` MIME parts so they don't render as body text.
2. **Phase 2 (Gmail read)**: Parse reaction MIME parts during sync, populate `message_reactions`, aggregate and display emoji chips below messages in threads. Hide reaction-only messages from thread list.
3. **Phase 3 (Exchange read)**: Fetch `ReactionsCount` and `OwnerReactionType` extended properties. Display the owner's reaction and total count. Defer full `ReactionsSummary` binary parsing.
4. **Phase 4 (Gmail write)**: Send reaction emails with correct MIME structure.
5. **Phase 5 (Exchange write)**: Blocked on Microsoft providing a public API. Do not reverse-engineer.
