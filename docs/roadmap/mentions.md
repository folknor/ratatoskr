# @Mentions

**Tier**: 2 — Keeps users from going back
**Status**: ⚠️ **Rethink needed** — Phase 1 backend (DB table, `is_mentioned` sync, HTML correlation, send mention metadata) was implemented but is unnecessary. @mentions in email are fundamentally a compose-time feature: insert `@Name` text in the body and add the person to To/CC. The Exchange mention metadata (`mentionsPreview`, `mentions` collection, beta API) is a nice-to-have highlight on the recipient side but not worth the complexity. All Phase 1 backend code should be removed. The only work needed is compose @-autocomplete (Phase 2).

---

- **What**: `@User` in email body, recipient gets the message auto-flagged
- **Dependency**: Contacts & Groups sync (Tier 1)

## Cross-provider behavior

| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `mentions` collection on message | Sync mention metadata, auto-flag mentioned user's copy |
| Gmail API | Nothing | Local-only: detect @-patterns in body, no server-side flagging |
| JMAP | Nothing | Local-only |
| IMAP | Nothing | Local-only |

## Pain points

- Display: Exchange stores mentions as structured metadata separate from the body HTML. The body contains the display text ("@John Smith") but the `mentions` collection has the resolved email/user ID. Need to correlate the two for highlighting.
- Compose: need @-autocomplete that triggers on `@` character in the compose editor, searches unified contacts, and inserts both the display text and the mention metadata (for Exchange accounts).
- Non-Exchange accounts: can still insert "@John Smith" text in the body (it's just text), but there's no server-side flagging. The recipient's client won't auto-flag it. Acceptable degradation — the visual cue in the body is still useful.
- Parsing incoming @mentions from non-Exchange senders: some people manually type "@Name" in emails. No metadata to parse — could attempt heuristic matching against contacts, but likely not worth the false positives.

## Work

@-autocomplete in compose: user types `@`, contact picker appears, selecting a contact inserts `@Display Name` in the body and adds them to To/CC if not already a recipient. Same behavior across all providers. The Exchange mention metadata (beta API `mentions` array) is not worth the complexity — skip it.

**Dead code to remove:** `crates/core/src/mentions.rs`, `crates/graph/src/mentions.rs`, `is_mentioned` column, `mentions` table (migration v40), `mentionsPreview` sync in `crates/graph/src/parse.rs`, mention metadata in `crates/graph/src/ops/send.rs`.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for the iced (pure Rust) rewrite. All provider interactions are raw HTTP via `reqwest` (Graph) or `jmap-client` (JMAP). Compose editor is iced's `text_editor` widget (plain text only). This feature depends on Contacts & Groups sync being complete (Tier 1).

---

### 1. Exchange Graph API Mentions — Data Model

#### API availability: beta only

The `mention` resource type and all associated APIs exist **exclusively in the Graph API `/beta` endpoint**. The v1.0 `message` resource has no `mentionsPreview` property, no `mentions` navigation property, and no mention-related filter capabilities. The v1.0 message JSON schema simply does not include mentions at all.

This is a significant constraint. Microsoft's `/beta` APIs carry an explicit warning: "APIs under the `/beta` version in Microsoft Graph are subject to change. Use of these APIs in production applications is not supported." Mentions have been in beta since at least 2016 (the Graph docs example timestamps show July 2016) — over nine years without promotion to v1.0. This suggests Microsoft may consider the feature stable enough to maintain but not important enough to stabilize.

**Architecture implication**: We must use the beta endpoint (`https://graph.microsoft.com/beta/`) for all mention operations. The rest of our Graph integration can use v1.0. Need to maintain beta-awareness in the reqwest call layer and accept the risk of breaking changes.

#### The `mention` resource

Each mention is a separate object in the `mentions` navigation property on a message:

```json
{
  "id": "138f4c0a-1130-4776-b780-bf79d73abb3f",
  "mentioned": {
    "name": "Dana Swope",
    "address": "danas@contoso.com"
  },
  "mentionText": null,
  "createdBy": {
    "name": "Samantha Booth",
    "address": "samanthab@contoso.com"
  },
  "createdDateTime": "2016-07-21T07:40:20.152Z",
  "serverCreatedDateTime": "2016-07-21T07:40:20.152Z",
  "deepLink": null,
  "application": null,
  "clientReference": null
}
```

Key observations:
- **`mentioned`** (`emailAddress`): The person who was @mentioned. Has `name` and `address`.
- **`createdBy`** (`emailAddress`): The person who made the mention (the sender).
- **`mentionText`**: Documented as optional, but in practice **always null for messages**. The docs explicitly say: "Not used and defaulted as null for message. To get the mentions in a message, see the bodyPreview property of the message instead." This is a dead field.
- **`application`**, **`clientReference`**, **`deepLink`**, **`serverCreatedDateTime`**: All documented as "not used and defaulted as null for message." These fields exist because `mention` is a generic resource type shared with other Graph entities.

The only meaningful fields for email mentions are: `id`, `mentioned.name`, `mentioned.address`, `createdBy.name`, `createdBy.address`, and `createdDateTime`.

#### The `mentionsPreview` property

On the message resource (beta only):

```json
"mentionsPreview": {
  "isMentioned": true
}
```

`mentionsPreview.isMentioned` is a `Boolean` indicating whether the signed-in user (the mailbox owner) is mentioned in this message. This is the "was I @mentioned?" flag. The server sets it automatically. It is **not** a general-purpose "does this message contain any mentions" indicator — it is scoped to the authenticated user.

Returned by default on `GET /me/messages` (no `$expand` needed). Read-only.

---

### 2. Graph API Operations for Mentions

#### Reading mentions on existing messages

**Get mentions for a specific message** — expand the `mentions` navigation property:
```
GET /beta/me/messages/{id}?$expand=mentions
```

The `mentions` property is NOT returned by default. Must use `$expand`. Returns the full mention array with all fields.

**Filter messages where I am mentioned** — use `$filter` on `mentionsPreview`:
```
GET /beta/me/messages?$filter=mentionsPreview/isMentioned eq true&$select=subject,sender,receivedDateTime,mentionsPreview
```

This is an efficient server-side filter. No need to fetch all messages and check locally.

**Gotcha**: `$filter` on `mentionsPreview/isMentioned` is only available in the beta endpoint. Cannot combine with arbitrary `$orderby` — the docs warn about `InefficientFilter` errors when filter/orderby properties conflict.

#### Creating mentions when sending

Include mentions in the `mentions` array when calling `POST /beta/me/sendMail`:

```json
{
  "message": {
    "subject": "Project kickoff",
    "toRecipients": [{
      "emailAddress": { "name": "Samantha Booth", "address": "samanthab@contoso.com" }
    }],
    "mentions": [{
      "mentioned": {
        "name": "Dana Swope",
        "address": "danas@contoso.com"
      }
    }]
  }
}
```

Only `mentioned` (with `name` and `address`) is required per mention object. The server populates `createdBy`, `createdDateTime`, and `id` automatically.

**Gotcha**: The mention metadata and the body HTML are **separate concerns**. Including a mention in the `mentions` array does NOT automatically insert "@Dana Swope" into the body HTML. The client must do both: (a) put the `@Name` text in the body HTML, and (b) include the corresponding `mentioned` entry in the `mentions` array. If you include the metadata but not the body text, the mention is invisible. If you include the body text but not the metadata, it's just plain text with no server-side flagging.

**MIME limitation**: Mentions can only be sent via JSON format (`sendMail` with `Content-Type: application/json`). MIME-format sends do not support the `mentions` property.

#### Deleting a mention

```
DELETE /beta/me/messages/{message-id}/mentions/{mention-id}
```

Removes a mention from a received message. This is useful if a user wants to clear the "mentioned" flag. Returns 204 No Content.

---

### 3. HTML Body Correlation

#### How Exchange represents mentions in HTML

From the Graph API documentation, the HTML body for a message with mentions looks like:

```html
<html><head></head><body><p>
  <a href="mailto:danas@contoso.com">@Dana Swope</a>,
  <a href="mailto:randiw@contoso.com">@Randi Welch</a>,
  forgot to mention, I will be away this weekend.
</p></body></html>
```

Key findings:
- **Mentions are `<a href="mailto:...">` tags** in the Graph API response. Not `<span>` elements, not custom data attributes — just standard `mailto:` links with the display text prefixed by `@`.
- **The display text** is `@{DisplayName}` (e.g., "@Dana Swope").
- **The href** is `mailto:{email}` (e.g., "mailto:danas@contoso.com").
- **No custom classes, IDs, or data attributes** are visible in the Graph API examples.

#### Correlation strategy

To correlate the structured `mentions` array with the body HTML:

1. Parse the HTML body (using an HTML parser like `scraper` or `lol_html`)
2. Find all `<a>` tags where `href` starts with `mailto:`
3. Extract the email address from the href
4. Match against the `mentions` array by `mentioned.address`
5. If matched, render with mention styling (highlight, different color, etc.)

This is reliable because:
- The email address in the `mailto:` link uniquely identifies the mentioned person
- The `mentions` array provides authoritative confirmation that this is a real mention (not just someone who typed a `mailto:` link)
- False positives are eliminated: only `mailto:` links whose address appears in the `mentions` array get mention styling

---

### 4. Auto-Flagging Behavior

When a user is @mentioned in a message sent via Exchange:

1. The sender includes the `mentions` array in the `sendMail` request
2. Exchange server processes the mention metadata
3. On the **recipient's** copy of the message, Exchange sets `mentionsPreview.isMentioned = true`
4. Outlook clients detect `isMentioned` and display a visual indicator (the "@" icon in the message list, highlighting in the reading pane)

**This is entirely server-side.** There is no client-side rule or flag manipulation needed. The server does the work of:
- Resolving which recipients match which mentions (by email address)
- Setting the `isMentioned` property on the recipient's copy
- Making the message filterable via `$filter=mentionsPreview/isMentioned eq true`

**There is no `isFlag` or `followupFlag` involvement.** Mentions do NOT set the message's `flag` property. They are a separate signaling mechanism.

**Gotcha**: The mentioned person must be a recipient (to, cc, or bcc) of the message for `isMentioned` to be set on their copy. If you @mention someone who is not a recipient, the mention metadata exists on the sender's copy but the mentioned person never sees it.

---

### 5. Rich Text Editing in iced — Compose Experience

#### Current state of iced text editing

The `iced::widget::text_editor` widget provides multi-line plain text editing with cursor management, selection, and basic edits. **No rich text support whatsoever.** There is no concept of inline spans, styled ranges, embedded widgets, or mixed content.

#### @-autocomplete implementation

**Option A: Plain-text compose with @-autocomplete popup**
1. User types `@` in the `text_editor`
2. Application detects `@` at cursor position by monitoring `Action` events
3. Show a floating autocomplete overlay (iced `container` positioned near cursor)
4. User selects a contact from the list (FTS5 query against local contacts DB)
5. Replace `@partial_input` with `@Display Name` in the text editor via `Edit::Paste`
6. Store the mention metadata (name, email) in an application-side `Vec<MentionDraft>` associated with the compose state
7. On send: generate HTML body converting `@Display Name` text to `<a href="mailto:email">@Display Name</a>`, and populate the `mentions` array for Exchange accounts

**Recommendation**: Option A. Plain-text compose with overlay autocomplete. The cursor position can be tracked via `text_editor`'s `Cursor` and the `@` trigger detected by examining the current line content.

#### @-trigger detection

Key UX details:
- Trigger on `@` only when preceded by whitespace or start of line (avoid triggering on email addresses like `user@example.com`)
- Dismiss on: escape, cursor moves away from the `@` position, two consecutive spaces after `@`
- Insert on: enter/tab/click selects the contact, replaces `@partial` with `@Full Name`
- The autocomplete popup should show: display name, email address, and avatar (from contacts DB)

---

### 6. Non-Exchange Fallback Behavior

For Gmail API, JMAP, and IMAP accounts, there is no server-side mention support.

| Aspect | Exchange (Graph) | Gmail / JMAP / IMAP |
|--------|-----------------|---------------------|
| Compose: @-autocomplete | Yes, inserts text + stores mention metadata | Yes, inserts text only |
| Send: mention metadata | Included in `mentions` array via beta API | Not applicable |
| Send: body HTML | `<a href="mailto:...">@Name</a>` | `<a href="mailto:...">@Name</a>` (same markup, cosmetic only) |
| Receive: server flags | `mentionsPreview.isMentioned` set by server | Nothing |
| Receive: filter | `$filter=mentionsPreview/isMentioned eq true` | Not possible server-side |

**Recommendation**: Still do @-autocomplete for non-Exchange accounts (helps users pick the right contact). On the display side, do NOT attempt to heuristically detect mentions in received messages for non-Exchange accounts — the false positive rate is too high.

---

### 7. Local Data Model

#### mentions table

```sql
CREATE TABLE mentions (
    message_id   TEXT NOT NULL,
    account_id   TEXT NOT NULL,
    mention_id   TEXT NOT NULL,       -- Graph beta mention ID
    mentioned_name    TEXT NOT NULL,
    mentioned_address TEXT NOT NULL,
    created_by_name    TEXT,
    created_by_address TEXT,
    created_at   TEXT,                -- ISO 8601
    PRIMARY KEY (message_id, account_id, mention_id),
    FOREIGN KEY (message_id, account_id) REFERENCES messages(id, account_id)
);

CREATE INDEX idx_mentions_address ON mentions(mentioned_address);
```

#### is_mentioned column on messages

```sql
ALTER TABLE messages ADD COLUMN is_mentioned INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_messages_is_mentioned ON messages(account_id, is_mentioned)
    WHERE is_mentioned = 1;
```

This denormalized boolean enables the "Messages mentioning me" filter view without joining to the mentions table. Set during sync when `mentionsPreview.isMentioned == true`. Only meaningful for Exchange accounts.

#### Sync strategy

During Exchange message sync (beta endpoint):
1. `GET /beta/me/messages?$select=...,mentionsPreview` — extract `isMentioned` and store in `messages.is_mentioned`
2. For messages where `is_mentioned = true` (or on demand when viewing), fetch full mention details: `GET /beta/me/messages/{id}?$expand=mentions`
3. Upsert mention records into the `mentions` table

**Do not eagerly expand mentions on every message during bulk sync.** The `$expand=mentions` adds a sub-request per message and significantly increases response size. Lazy-load full mention details when the user opens a message with `is_mentioned = true`.

---

### 8. What Outlook Does — Reference UX

**Compose**: User types `@` in the body, autocomplete dropdown shows contacts, selecting inserts a highlighted `@Name` and auto-adds the person to To: if not already a recipient.

**Reading pane**: Messages where the user is @mentioned show an `@` icon in the message list. The mention text is highlighted in the reading pane. Users can filter by "Mentioned" to see only messages where they are @mentioned.

**Auto-add to recipients**: When you @mention someone, Outlook automatically adds them to the To: line. We should replicate this.

---

### 9. Implementation Plan

**Phase 1: Display (read-only)** — ❌ **Remove.** Implemented but unnecessary. @mentions in email are a compose-time feature (insert text + add to recipients), not a display-time feature. The Exchange beta API mention metadata adds complexity for minimal user value. Code to remove: `crates/core/src/mentions.rs`, `crates/graph/src/mentions.rs`, `is_mentioned` column + `mentions` table (migration v40), `mentionsPreview` extraction in `crates/graph/src/parse.rs`, mention metadata in `crates/graph/src/ops/send.rs`.

**Phase 2: Compose** — This is the only phase that matters.
1. Implement @-autocomplete trigger detection in the compose `text_editor`
2. Show floating contact picker overlay, querying FTS5 contacts
3. On selection, insert `@Display Name` text in the body
4. Auto-add mentioned person to To: if not already a recipient
5. On send, convert `@Display Name` to `<a href="mailto:email">@Display Name</a>` in HTML body (cosmetic, works across all providers)

**Phase 3: Polish** — ❌ **Remove.** Mention deletion, count badge, and forward handling were all tied to the Exchange beta API metadata. Not needed.

---

### 10. Risk Assessment

| Risk | Severity | Mitigation |
|------|----------|------------|
| Mentions API stuck in beta forever | Medium | Already works in production Outlook. If deprecated, mentions degrade to cosmetic-only. No data loss. |
| Beta API breaking change | Low | Unchanged since 2016. Monitor Graph changelog. |
| No rich text compose | Medium | Plain text `@Name` insertion works. Visual experience in compose is worse than Outlook but functionally complete. |
| Contacts DB incomplete at mention time | Medium | Allow manual email entry as a mention target. |
