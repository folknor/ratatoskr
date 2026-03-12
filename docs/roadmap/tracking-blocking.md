# Tracking Pixel / Read Receipt Blocking

**Tier**: 1 — Blocks switching from Outlook
**Status**: ⚠️ **Mostly done** — Remote image blocking is fully implemented: blocked by default, CSP enforcement on iframe, per-sender allowlist (`image_allowlist` table), "load images" / "always load from sender" buttons. **Missing**: MDN (`Disposition-Notification-To`) suppression, per-account/per-sender read receipt policy.

---

- **What**: Block remote image loading by default (defeats tracking pixels), suppress MDN (Message Disposition Notification) headers
- **Scope**: Client-side only — identical implementation across all providers

## Pain points

- Blocking remote images breaks legitimate email layouts: newsletters, marketing emails, and even some corporate templates rely on remote images for logos, banners, formatting. Need a "load images for this message" toggle and a per-sender/per-domain allowlist.
- Read receipts (`Disposition-Notification-To` header): some corporate environments expect read receipts. Blocking them entirely may violate workplace expectations. Need a per-account or per-sender policy (auto-send, ask, never).
- Tracking pixels are invisible 1x1 images — but some "tracking" is done via uniquely-parameterized URLs on visible images. Blocking all remote images is the only reliable defense, but it's heavy-handed.
- AMP for Email: some senders use AMP emails that phone home. Treat AMP content as remote content and block by default.
- HTML email `<link>` tags and CSS `@import`: remote CSS is another tracking vector. Block external stylesheets, inline only.

## Work

Default-block remote images in HTML render, strip/suppress `Disposition-Notification-To`, per-sender allowlist, "load images for this message" one-shot button.
