# Scheduled Send

**Tier**: 2 — Keeps users from going back
**Status**: ✅ **Done (local)** — Full implementation: schedule dialog with presets (tomorrow 9am/1pm, Monday 9am) + custom date/time picker, `scheduled_emails` table with status tracking (pending/sending/sent/failed/cancelled), background sender service. Currently local-only (client holds and sends). **Missing**: server-native delegation to Exchange/Gmail/JMAP for reliability when client is offline.

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
- Time zones: user schedules for "9 AM Monday" — whose Monday? Need explicit time zone handling in the schedule picker. Display in local time, store as UTC, convert to recipient's time zone for preview ("arrives ~9 AM EST for recipient").
- Cancellation: for server-side scheduled send, need to support cancel/reschedule. For Exchange this means deleting the deferred message from Drafts. For local fallback, just remove from the local queue.
- Scheduled view: need a "Scheduled" mailbox/view showing all pending scheduled messages across accounts, with ability to edit, reschedule, or cancel. This is a virtual folder, not a real server-side mailbox.
- Multi-account: user has Exchange (server-side scheduling) and IMAP (local scheduling). The UI should be identical. But the reliability characteristics differ — worth a subtle indicator?

## Work

Schedule picker in compose, server-native send for Exchange/Gmail/JMAP, local timer+queue for IMAP, "Scheduled" virtual view, cancel/reschedule support.
