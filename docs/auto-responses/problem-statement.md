# Auto-Responses (Out-of-Office / Vacation Replies)

## Overview

Auto-responses let users configure automatic reply messages that the **server sends on their behalf** while they're away. This is a server-side feature - the client configures it, the server executes it. The user can close Ratatoskr, shut down their computer, and replies still go out.

Every major provider has a full read/write API for this. Unlike reactions or signatures, there is no provider gap - this is one of the few features with genuine cross-provider parity.

## Why this matters

Enterprise Exchange users rely on out-of-office replies daily. It's a settings-level feature that users expect to find immediately. Missing it blocks switching from Outlook for anyone who travels or takes time off.

## Cross-Provider Support

| Provider | API | Read | Write | Server executes | Separate internal/external messages |
|----------|-----|------|-------|-----------------|-------------------------------------|
| Exchange (Graph) | `GET/PATCH /me/mailboxSettings/automaticRepliesSetting` (v1.0) | Yes | Yes | Yes | Yes |
| Gmail | `users.settings.getVacation` / `users.settings.updateVacation` | Yes | Yes | Yes | No (one message, but `restrictToContacts`/`restrictToDomain` flags) |
| JMAP | `VacationResponse/get` / `VacationResponse/set` (RFC 8621) | Yes | Yes | Yes | No (one message) |
| IMAP | ManageSieve (RFC 5804) + Sieve vacation (RFC 5230) | Server-dependent | Server-dependent | Yes, if Sieve supported | Server-dependent |

### Exchange (Graph) - `automaticRepliesSetting`

The most complete model. Part of `mailboxSettings` (v1.0, not beta).

```
GET /me/mailboxSettings/automaticRepliesSetting
PATCH /me/mailboxSettings
```

Properties:
- **`status`**: `"disabled"` | `"alwaysEnabled"` | `"scheduled"`
- **`externalAudience`**: `"none"` | `"contactsOnly"` | `"all"`
- **`internalReplyMessage`**: HTML string - sent to people inside the org
- **`externalReplyMessage`**: HTML string - sent to people outside the org
- **`scheduledStartDateTime`**: `{ "dateTime": "2026-03-25T08:00:00", "timeZone": "Europe/London" }`
- **`scheduledEndDateTime`**: same format

Permission: `MailboxSettings.ReadWrite` (already required for other mailbox settings).

Key behaviors:
- Server sends replies even when Outlook/client is closed
- Scheduling is timezone-aware (dateTimeTimeZone resource)
- Exchange auto-disables when `scheduledEndDateTime` passes
- Internal/external distinction is org-boundary aware (same tenant = internal)

### Gmail - `VacationSettings`

```
GET  https://gmail.googleapis.com/gmail/v1/users/me/settings/vacation
PUT  https://gmail.googleapis.com/gmail/v1/users/me/settings/vacation
```

Properties:
- **`enableAutoReply`**: boolean
- **`responseSubject`**: optional string
- **`responseBodyPlainText`**: string
- **`responseBodyHtml`**: string (if both provided, HTML is used)
- **`startTime`**: epoch milliseconds (optional - if omitted, starts immediately)
- **`endTime`**: epoch milliseconds (optional - if omitted, runs until manually disabled)
- **`restrictToContacts`**: boolean - only reply to known contacts
- **`restrictToDomain`**: boolean - only reply to same-domain senders

Permission: `https://www.googleapis.com/auth/gmail.settings.basic` (already used for signatures).

Key behaviors:
- No separate internal/external messages - one message for everyone
- `restrictToContacts` + `restrictToDomain` are the audience controls
- Gmail deduplicates: sends at most one reply to each sender per vacation period

### JMAP - `VacationResponse` (RFC 8621 §7)

```
VacationResponse/get
VacationResponse/set
```

Properties:
- **`isEnabled`**: boolean
- **`fromDate`**: UTCDate (nullable - if null, effective immediately)
- **`toDate`**: UTCDate (nullable - if null, runs until disabled)
- **`subject`**: string (nullable - server generates default if null)
- **`textBody`**: string (nullable)
- **`htmlBody`**: string (nullable)

Capability: `urn:ietf:params:jmap:vacationresponse`

Key behaviors:
- Singleton object - one per account, always exists, only `VacationResponse/set` to update
- Server implements RFC 5230 Sieve vacation semantics (dedup, rate limiting)
- `jmap-client` crate: **does not appear to support VacationResponse** - will need manual JMAP request or crate extension

### IMAP - ManageSieve (RFC 5804) + Sieve vacation (RFC 5230)

No standard IMAP command for vacation replies. The mechanism is:

1. Connect to the ManageSieve port (typically 4190)
2. Upload a Sieve script containing a `vacation` action
3. Server executes the script on incoming mail

This requires:
- Server supports Sieve + vacation extension (Dovecot, Cyrus, Stalwart do; many others don't)
- ManageSieve port accessible and authenticated
- Generating valid Sieve syntax

**Rust ManageSieve crates:** None of note. Would need raw protocol implementation or shelling out.

**Recommendation:** IMAP auto-responses are a stretch goal. The three API providers (Exchange, Gmail, JMAP) cover the vast majority of users. For IMAP accounts without ManageSieve, show a message: "Your email server doesn't support auto-replies through Ratatoskr. Configure auto-replies in your server's webmail interface."

## Unified Data Model

Despite API differences, the core model is the same everywhere:

```
AutoResponseConfig {
    enabled: bool,
    start_date: Option<DateTime>,      // None = effective immediately
    end_date: Option<DateTime>,         // None = until manually disabled
    internal_message_html: Option<String>,  // Exchange only; others use single message
    external_message_html: Option<String>,  // All providers have at least this
    external_audience: ExternalAudience,    // None | ContactsOnly | All
}

enum ExternalAudience {
    None,           // Exchange: externalAudience=none
    ContactsOnly,   // Exchange: contactsOnly, Gmail: restrictToContacts
    All,            // Exchange: all, Gmail: default
}
```

For providers without internal/external distinction (Gmail, JMAP): `internal_message_html` is ignored, `external_message_html` is used as the single reply message.

For providers without audience control (JMAP): `ExternalAudience` is stored locally but not enforced server-side.

## UI

Settings panel, likely under each account's settings or as a top-level "Auto-replies" section:

1. **Enable/disable toggle**
2. **Schedule** - optional start and end dates with time pickers. "Send replies during this period" or "Send replies until I turn this off"
3. **Message editor** - rich text (reuse compose editor or a simplified version). For Exchange accounts, show two tabs: "Inside my organization" / "Outside my organization". For other providers, single message field.
4. **Audience** - "Reply to everyone" / "Reply to contacts only" / "Don't reply to external senders". Map to provider-specific controls.
5. **Status indicator** - when auto-replies are active, show a persistent indicator in the status bar or sidebar (like Outlook does).

## Implementation Suggestions

### Phase 1: Exchange + Gmail (highest value)

These two cover the majority of enterprise and consumer users.

1. **Data types** in `crates/core/` - `AutoResponseConfig`, `ExternalAudience` enum, per-provider serialization
2. **Exchange read/write** in `crates/graph/` - `GET/PATCH /me/mailboxSettings/automaticRepliesSetting`. Straightforward JSON - no extended properties, no beta endpoint. Already have `MailboxSettings.ReadWrite` permission.
3. **Gmail read/write** in `crates/gmail/` - `GET/PUT users/me/settings/vacation`. Already have `gmail.settings.basic` scope (used for signatures).
4. **Settings UI** in `crates/app/` - per-account auto-reply editor in account settings. Toggle, date pickers, message editor, audience selector. Show internal/external tabs only for Exchange accounts.
5. **Status bar indicator** - when any account has active auto-replies, show an indicator.

### Phase 2: JMAP

6. **JMAP read/write** - `VacationResponse/get` and `VacationResponse/set`. If `jmap-client` doesn't support it, build manual JMAP method calls using the existing HTTP transport. The request format is simple.

### Phase 3: IMAP (stretch)

7. **ManageSieve** - only if there's demand. Requires ManageSieve protocol implementation, Sieve script generation, and capability detection. Most IMAP users who need auto-replies already configure them via webmail.

### Sync on account add

On first account setup, fetch the current auto-reply state and display it. If the user already has auto-replies configured (e.g., they set it up in Outlook before switching), Ratatoskr should show the existing configuration, not a blank form.
