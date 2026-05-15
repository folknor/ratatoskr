# Read Receipts

Status snapshot: 2026-05-15.

Outgoing read-receipt requests and incoming MDN-request detection are fully shipped across all four providers. The MDN *response* infrastructure (RFC 8098 message builder, $MDNSent keyword tracking, scoped policy resolution) is built but not yet wired into the mark-as-read code path. UI for policy and per-message prompts is deferred.

## Outgoing: always request

Every outgoing message includes a read-receipt request:

- IMAP / Gmail / JMAP: `common::headers::inject_read_receipt_header_base64url` adds `Disposition-Notification-To: <sender>` to the raw RFC 2822 bytes before send. Sender is extracted from the `From:` header.
- Graph: the request is the native `is_read_receipt_requested: true` field on the Graph draft (set in `crates/graph/src/ops/send.rs::mime_to_graph_message`), since Graph's send API doesn't take raw MIME.

No toggle, no setting, no UI. The product stance is that requesting is acceptable because the recipient retains full control of whether to respond, but we should not frame it as having zero privacy implications.

## Incoming: detection

All four providers parse the incoming `Disposition-Notification-To` header into a `mdn_requested: bool` field on the parsed message. The DB schema has a matching `messages.mdn_requested` column. JMAP additionally requests the header explicitly via `Property::Header` so it survives the get/changes flow.

`db::queries_extra::mdn::is_mdn_requested_graph` is the lookup helper (the name predates the wiring and is provider-agnostic in practice; it just reads the column).

## Incoming: response

Wired into `crates/service/src/actions/mark_read.rs`: after a successful `read=true` provider mark_read, `mdn_send::send_mdn_responses` enumerates pending requests in the thread, resolves the per-sender policy, and on `Always` builds + sends an MDN via `ProviderOps::send_email`, marks `mdn_sent` locally, and pushes the server-side keyword via `ProviderOps::mark_mdn_sent`. `Ask` is treated as `Never` until the prompt UI ships. All steps soft-fail (log + continue) to keep the user-visible mark-read action atomic.

### Policy resolution

`db::queries_extra::mdn::resolve_read_receipt_policy(conn, account_id, sender_email)` returns `ReadReceiptPolicy::{Always, Ask, Never}` via most-specific-wins lookup:

1. `sender:{exact_email}` row in `read_receipt_policy` table
2. `domain:{domain}` row
3. account-level row (scope = `account`)
4. global default from `settings.default_read_receipt_policy`
5. hard-coded fallback: `Never`

The `read_receipt_policy` table and the `default_read_receipt_policy` setting key already exist in the schema.

### MDN message builder

`core::mdn::build_mdn_message(original_from, original_message_id, recipient_email, recipient_name, is_manual)` returns RFC 8098-compliant `multipart/report; report-type=disposition-notification` raw MIME bytes. Picks `manual-action/MDN-sent-manually` or `automatic-action/MDN-sent-automatically` based on the flag.

### Sent-flag tracking

- `mark_mdn_sent_local(conn, account_id, message_id)`: sets `messages.mdn_sent = 1`.
- `is_mdn_already_sent(conn, account_id, message_id)`: local DB check.
- `ProviderOps::mark_mdn_sent(ctx, message_id)`: server-side keyword sync. IMAP looks up `(imap_folder, imap_uid)`, opens a session, sets `$MDNSent` via `UID STORE +FLAGS` (silent no-op if the server doesn't allow custom keywords). JMAP sends `Email/set` with `$mdnsent` true. Gmail and Graph use the default no-op trait impl - Graph's `isReadReceiptRequested` is read-only and Gmail has no equivalent keyword.

## Outstanding work

### Per-message prompt UI (deferred)

`Ask` policy needs a banner/modal in the reading pane. Pending the read-pane redesign.

### Settings UI (deferred)

A preferences panel for `default_read_receipt_policy` and per-sender / per-domain entries in `read_receipt_policy`. Until the UI ships, the policy can be set via direct `settings` table inserts (or in `dev-seed` for local development).

## Not doing

- Tracking pixels (invisible 1x1 images in outgoing HTML). Deceptive; violates recipient privacy. Superhuman's approach is explicitly rejected.
- IP / location logging from read receipts.
- Any tracking the recipient isn't aware of.
