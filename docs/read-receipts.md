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

## Incoming: response infrastructure (built, unwired)

Everything below exists in code but has zero non-test consumers as of 2026-05-15.

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
- `mark_mdn_sent_imap(session, folder, uid)`: sets the `$MDNSent` IMAP keyword via `UID STORE +FLAGS`. Silent no-op if the server doesn't permit custom keywords.
- `mark_mdn_sent_jmap(client, message_id)`: sets the `$mdnsent` JMAP keyword via `Email/set`.
- `is_mdn_sent_imap(session, folder, uid)`: `UID SEARCH KEYWORD $MDNSent`.
- `is_mdn_already_sent(conn, account_id, message_id)`: local DB check.
- Graph has no server-side equivalent (`isReadReceiptRequested` is read-only); we track sent status via `mark_mdn_sent_local` only.

## Outstanding work

### Wire the response (next backend slice)

The MDN response chain has all the parts but no caller. Concretely:

1. Hook into `crates/service/src/actions/mark_read.rs` after `mark_read_local` succeeds with `read == true`.
2. For each message in the thread with `mdn_requested && !mdn_sent`:
   - Read `from_address`, `headers.Message-ID`, account `email` + `display_name`.
   - `resolve_read_receipt_policy(conn, account_id, sender_email)`.
   - On `Always`: `build_mdn_message(...)`, send via provider's send path (`send_raw_email` for IMAP+SMTP, Gmail/JMAP raw send, Graph draft route), then `mark_mdn_sent_local` plus the provider-specific `mark_mdn_sent_*`.
   - On `Ask`: stash a pending-prompt record (UI work, deferred); for now treat as `Never`.
   - On `Never`: do nothing.
3. Failures must be soft: provider send errors should not unmark-read; log and move on. The receipt is best-effort.

### Per-message prompt UI (deferred)

`Ask` policy needs a banner/modal in the reading pane. Pending the read-pane redesign.

### Settings UI (deferred)

A preferences panel for `default_read_receipt_policy` and per-sender / per-domain entries in `read_receipt_policy`. Until the UI ships, the policy can be set via direct `settings` table inserts (or in `dev-seed` for local development).

## Not doing

- Tracking pixels (invisible 1x1 images in outgoing HTML). Deceptive; violates recipient privacy. Superhuman's approach is explicitly rejected.
- IP / location logging from read receipts.
- Any tracking the recipient isn't aware of.
