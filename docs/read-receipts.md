# Read Receipts

## Outgoing: Always Request

Every outgoing message includes the `Disposition-Notification-To` header, set to the sender's address. No toggle, no setting, no UI — it's always on.

This is the standards-based mechanism (RFC 8098 / RFC 3798). The header is a *request*, not a demand. The recipient's mail client decides whether to honor it:

- Most clients silently ignore it
- Some prompt the recipient ("Alice requested a read receipt. Send one?")
- Some auto-respond based on the recipient's settings

There is no privacy concern for the sender or recipient. The recipient is always in control. This is not a tracking pixel — it's an explicit, visible protocol feature.

## Incoming: TBD

When Ratatoskr receives a message with `Disposition-Notification-To`, we need to decide how to respond. Options:

1. **Silently decline** (default) — privacy-first, don't reveal read status
2. **Prompt per-message** — "Alice requested a read receipt. Send one?" with remember-per-sender option
3. **Auto-respond** — always send receipts back (transparent, reciprocal)
4. **Per-sender setting** — auto-respond for contacts, decline for unknown senders

Leaning toward option 1 (silently decline) as default with a global or per-sender override. But this is a future design decision — the outgoing side ships first with zero UI surface.

## Implementation

**Outgoing** (compose/send path):
- Add `Disposition-Notification-To: <sender@address>` header to all outgoing messages in the provider send functions
- No UI changes required

**Incoming** (sync/display path — future):
- Detect `Disposition-Notification-To` header on received messages
- Store the request in message metadata
- When the message is read, apply the configured policy (decline/prompt/auto)
- If responding, send a Message Disposition Notification (MDN) per RFC 8098

## Not Doing

- Tracking pixels (invisible 1x1 images in outgoing HTML). This is deceptive and violates the recipient's privacy. Superhuman's approach is explicitly rejected.
- IP/location logging from read receipts.
- Any form of tracking that the recipient isn't aware of.

## Ecosystem Patterns

Read receipts is primarily a protocol-layer feature. The outgoing side (Phase 1) requires zero UI work — it is a header addition in the provider send path. Overlap with the iced ecosystem survey is minimal.

| Requirement | Primary Source | Applicability |
|---|---|---|
| Async MDN send | pikeru/rustcast subscriptions | Low — generic async; real work is RFC 8098 compliance |
| Receipt policy settings UI | bloom config shadow | Medium — relevant if/when a settings panel is built for incoming receipt policy |
| Global policy storage | rustcast TOML config defaults | Low — single enum field (`Decline`/`Prompt`/`Auto`/`PerSender`) |
| Per-message receipt prompt | trebuchet Component trait | Medium — isolates prompt state from reading pane, keeping the prompt lifecycle self-contained |

The heavy lifting is RFC 8098 compliance, database schema for storing receipt requests, and provider integration for sending MDNs — none of which the iced ecosystem survey addresses. The only meaningful UI touchpoints are the future incoming-receipt prompt (a modal or inline banner when reading a message) and the policy setting in preferences, both of which are deferred until Phase 2.
