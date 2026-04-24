# Scheduled Send

**Tier**: 2 - Keeps users from going back
**Status**: ✅ **Done** - Full implementation with server-native delegation. `crates/core/src/scheduled_send.rs`: delegation routing (`determine_send_delegation_for_account` routes Exchange/JMAP to server, Gmail/IMAP to local), overdue handling (auto-send if <24h, flag for review if >24h). DB schema and queries in `crates/db/` (migrations, types) and `crates/core/src/db/queries_extra/compose.rs`. Exchange deferred delivery via `PidTagDeferredSendTime` extended property (`schedule_send`, `cancel_scheduled_send`, `reschedule_send` in `crates/graph/src/ops/mod.rs`). JMAP FUTURERELEASE via `EmailSubmission` with `HOLDUNTIL` parameter (`schedule_send_jmap`, `cancel_scheduled_send_jmap` in `crates/jmap/src/ops.rs`). Gmail/IMAP use local timer.

---

- **What**: Compose now, deliver later at a specified time

## Cross-provider behavior

| Provider | Native support | Mechanism |
|---|---|---|
| Exchange (Graph) | Deferred delivery via extended properties | Server holds message until send time |
| Gmail API | Native scheduled send | Server-side |
| JMAP | `EmailSubmission` with `sendAt` | Server holds until send time |
| IMAP/SMTP | Nothing | Client must hold and send |

## Pain points

- IMAP fallback: the client must keep the message and send it at the scheduled time. If the client is closed/offline at send time, the message doesn't go. Need to communicate this clearly ("this will only send if Ratatoskr is running at the scheduled time") or implement a send-on-next-wake queue.
- Time zones: user schedules for "9 AM Monday" - whose Monday? Need explicit time zone handling in the schedule picker. Display in local time, store as UTC, convert to recipient's time zone for preview ("arrives ~9 AM EST for recipient").
- Cancellation: for server-side scheduled send, need to support cancel/reschedule. For Exchange this means deleting the deferred message from Drafts. For local fallback, just remove from the local queue.
- Scheduled view: need a "Scheduled" mailbox/view showing all pending scheduled messages across accounts, with ability to edit, reschedule, or cancel. This is a virtual folder, not a real server-side mailbox.
- Multi-account: user has Exchange (server-side scheduling) and IMAP (local scheduling). The UI should be identical. But the reliability characteristics differ - worth a subtle indicator?

## Work

- ✅ DB schema with delegation columns (`delegation`, `remote_message_id`, `remote_status`, `timezone`, `from_email`, `error_message`, `retry_count`)
- ✅ Delegation routing - `determine_send_delegation_for_account` maps provider type to `SendDelegation` enum (`Local`/`Exchange`/`Jmap`)
- ✅ Exchange deferred delivery - `schedule_send` sets `PidTagDeferredSendTime` (0x3FEF) extended property; `cancel_scheduled_send` deletes draft; `reschedule_send` PATCHes timestamp
- ✅ JMAP FUTURERELEASE - `schedule_send_jmap` creates `EmailSubmission` with `HOLDUNTIL` parameter; `cancel_scheduled_send_jmap` sets `undoStatus` to `canceled`; checks `maxDelayedSend` capability
- ✅ Overdue handling - `check_overdue_scheduled_emails` classifies locally-delegated overdue emails: `SendNow` if <24h, `NeedsReview` if >24h; `process_overdue_emails` applies resolutions
- ✅ Gmail/IMAP local scheduling (no server API available)
- ⬚ Schedule picker UI (iced compose work)
- ⬚ "Scheduled" virtual folder view

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for the iced (pure Rust) rewrite. The existing local scheduler (`scheduled_emails` table + background sender) needs server-native delegation for Exchange and JMAP.

---

### 1. Exchange: Deferred Delivery via Graph API

Exchange supports server-side deferred delivery through MAPI extended properties exposed via the Graph API v1.0. There is no first-class `scheduledSendTime` field; instead, set `PidTagDeferredSendTime` (property tag `0x3FEF`) as a `singleValueLegacyExtendedProperty`.

```json
POST https://graph.microsoft.com/v1.0/me/messages
{
  "subject": "...",
  "body": { "contentType": "HTML", "content": "..." },
  "toRecipients": [{ "emailAddress": { "address": "..." } }],
  "singleValueExtendedProperties": [
    { "id": "SystemTime 0x3FEF", "value": "2026-03-13T09:00:00Z" }
  ]
}
```

This creates a draft with a deferred send time. When you call `POST /me/messages/{id}/send`, Exchange holds the message server-side and releases it at the specified time. Alternatively, use `POST /me/sendMail` with the extended properties inline.

**Relative delay approach**: Set `PidTagDeferredSendNumber` (`Integer 0x3FEB`, range 0-999) and `PidTagDeferredSendUnits` (`Integer 0x3FEC`, where 0=minutes, 1=hours, 2=days, 3=weeks).

**Cancellation**: Delete the message from Drafts before the deferred send time (`DELETE /me/messages/{id}`). Once Exchange releases it, cancellation is not possible.

**Reschedule**: PATCH the extended property with a new timestamp.

This works on Graph v1.0 (not beta-only). The property ID format is `"{GraphType} {PropTag}"` where GraphType is `SystemTime` and PropTag is `0x3FEF`.

---

### 2. Gmail API: No Native Scheduled Send

The Gmail API does **not** support scheduled send. `messages.send` and `drafts.send` send immediately; there is no `scheduledSendTime` parameter. Gmail's web UI "Schedule send" feature is implemented client-side and not exposed through the API.

Google Issue Tracker #140922183 has been open since 2019 and remains unresolved.

**Implication**: Gmail accounts must always use local scheduling. The UI should display a clear warning: "This message will only send if Ratatoskr is open at the scheduled time."

---

### 3. JMAP: `EmailSubmission` with FUTURERELEASE

RFC 8621 supports scheduled send through the SMTP FUTURERELEASE extension (RFC 4865) surfaced via JMAP's `EmailSubmission` object.

**Server capability**: `urn:ietf:params:jmap:submission` advertises `maxDelayedSend` (seconds). If 0, delayed send is not supported. `submissionExtensions` lists supported SMTP extensions.

**How it works**: Create an `EmailSubmission` with FUTURERELEASE parameters in the `envelope.mailFrom.parameters` field:
- `HOLDFOR=<seconds>` - relative delay
- `HOLDUNTIL=<RFC3339-UTC-datetime>` - absolute release time

**`sendAt` property**: `UTCDate`, immutable, server-set. The scheduled release time.

**`undoStatus` property**: `pending` (cancelable), `final` (relayed), `canceled` (will not deliver).

**Cancellation**: Update `undoStatus` to `canceled` via `EmailSubmission/set`. Server returns `cannotUnsend` error if already relayed.

**`jmap-client` crate support**: Exposes `email_submission_create`, `email_submission_create_envelope`, `email_submission_change_status`, `email_submission_destroy`, `send_at()`, and `undo_status()`. **Gap**: The `Address` type may not expose a `parameters` field for FUTURERELEASE parameters. May require lower-level request builder or patching `jmap-client`.

**Stalwart**: Advertises FUTURERELEASE and DELIVERBY as supported SMTP extensions.

---

### 4. SMTP FUTURERELEASE Extension (RFC 4865)

RFC 4865 defines FUTURERELEASE on SMTP submission ports (587). **Real-world adoption is extremely limited.** Postfix, Exim, and Sendmail do not implement it. Stalwart is the notable exception. For IMAP accounts connecting to arbitrary SMTP servers, FUTURERELEASE is not viable.

---

### 5. Local Scheduling Architecture

**In-process timer (recommended)**:
- Use `tokio::time::sleep_until` to wake at the next scheduled send time
- Maintain a priority queue of pending scheduled emails
- On each wake: query `scheduled_emails WHERE status = 'pending' AND scheduled_at <= now`, attempt send, update status
- On new schedule or cancel: recompute next wake time

**OS-level scheduling** (cron, systemd timers, Windows Task Scheduler) was considered and rejected - too complex to install/manage for a desktop app.

**Hybrid approach for reliability**:
1. In-process timer handles sending while the app is running
2. On app startup, immediately check for overdue scheduled emails and send them
3. Show a notification: "2 scheduled emails were sent (they were due while Ratatoskr was closed)"
4. If overdue by >24h, prompt user rather than sending silently

---

### 6. Cancellation Mechanics Per Provider

| Provider | Cancel mechanism | API call |
|---|---|---|
| Exchange | Delete deferred message from Drafts | `DELETE /me/messages/{id}` |
| Gmail | Remove from local queue | DB delete |
| JMAP | Set `undoStatus` to `canceled` | `EmailSubmission/set` update |
| IMAP/SMTP | Remove from local queue | DB delete |

---

### 7. Time Zone Handling

**Crate choice: `jiff`** (by BurntSushi).

- First-class IANA time zone support built in
- `Zoned` type represents timezone-aware instants
- DST-aware arithmetic: adding "1 day" across DST gives correct civil time
- Uses system timezone database on Unix, bundles on Windows

```rust
use jiff::{Zoned, ToSpan};
let now = Zoned::now();
let tomorrow_9am = now.date()
    .checked_add(1.day())?
    .at(9, 0, 0, 0)
    .to_zoned(now.time_zone().clone())?;
let utc_timestamp = tomorrow_9am.timestamp();
```

**Storage**: UTC unix timestamp (i64) in the database. Store IANA timezone ID alongside for display.

---

### 8. Data Model Changes

Required additions to the existing `scheduled_emails` table:

```sql
ALTER TABLE scheduled_emails ADD COLUMN delegation TEXT DEFAULT 'local';
  -- 'local' | 'exchange' | 'jmap'
ALTER TABLE scheduled_emails ADD COLUMN remote_message_id TEXT;
  -- Exchange: Graph message ID in Drafts. JMAP: EmailSubmission ID.
ALTER TABLE scheduled_emails ADD COLUMN remote_status TEXT;
  -- Exchange: 'deferred' | 'released'. JMAP: 'pending' | 'final' | 'canceled'.
ALTER TABLE scheduled_emails ADD COLUMN timezone TEXT;
  -- IANA timezone ID for display
ALTER TABLE scheduled_emails ADD COLUMN from_email TEXT;
  -- Sender address for send-as support
ALTER TABLE scheduled_emails ADD COLUMN error_message TEXT;
ALTER TABLE scheduled_emails ADD COLUMN retry_count INTEGER DEFAULT 0;
```

**Status flow by delegation type**:

- **Local** (Gmail/IMAP): `pending -> sending -> sent` or `pending -> sending -> failed`
- **Exchange**: `pending -> delegated -> sent` or `pending -> delegated -> cancelled`
- **JMAP**: `pending -> delegated -> sent` or `pending -> delegated -> cancelled`

---

### Provider Support Summary

| Capability | Exchange (Graph) | Gmail API | JMAP (Stalwart) | IMAP/SMTP |
|---|---|---|---|---|
| Server-side hold | Yes (extended property) | No | Yes (FUTURERELEASE) | No |
| Cancel | Delete from Drafts | N/A | `undoStatus = canceled` | N/A |
| Reschedule | PATCH extended property | N/A | Cancel + new submission | N/A |
| Max delay | Unlimited | N/A | Server-configured | N/A |
| Reliability | High (server holds) | Low (client must be running) | High (server holds) | Low (client must be running) |
