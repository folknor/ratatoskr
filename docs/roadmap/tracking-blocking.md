# Tracking Pixel / Read Receipt Blocking

**Tier**: 1 — Blocks switching from Outlook
**Status**: ⚠️ **Mostly done** — Remote image blocking is fully implemented: blocked by default, CSP enforcement on iframe, per-sender allowlist (`image_allowlist` table), "load images" / "always load from sender" buttons. MDN infrastructure is in place: `Disposition-Notification-To` header is detected during sync across all four providers and persisted as `mdn_requested` on messages; `read_receipt_policy` table exists with per-account/per-sender scoping; default policy is `never` (suppress silently). **Remaining**: UI for read receipt prompts and policy management. HTML sanitization pipeline (`sanitize_html_body()`) now implemented in core with css-inline + lol_html + ammonia.

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

## Research

### 1. HTML email rendering in iced — the core challenge

The Tauri implementation renders HTML email in a webview iframe with CSP headers blocking remote resources. In iced there is no webview. This is the single most architecturally significant decision for the iced migration because it determines how *all* email bodies are displayed.

#### Rendering backend options

Four viable approaches exist for embedding HTML rendering in an iced application, all accessible through the [`iced_webview_v2`](https://crates.io/crates/iced_webview) integration library (v0.1, published to crates.io):

| Backend | CSS support | JavaScript | Binary overhead | crates.io? | Maturity |
|---------|-------------|-----------|-----------------|------------|----------|
| **litehtml** (C++ with Rust bindings via `litehtml-sys`) | CSS2 + basic flexbox, no grid | None | Small (~2 MB) | Yes | Stable, production-quality for simple content |
| **Blitz** (DioxusLabs, pure Rust) | Flexbox, grid, tables, CSS variables, media queries via Stylo + Taffy | None | Moderate | Git dep only (not on crates.io) | Pre-alpha; ~3.4k GitHub stars, 28 contributors, aiming for beta end-2025, production 2026 |
| **Servo** (Mozilla/Linux Foundation) | Full HTML5/CSS3 | SpiderMonkey (unstable) | 50-150 MB | Git dep only | Experimental; SpiderMonkey crashes on heavy JS, no text selection API |
| **CEF** (Chromium Embedded Framework via Tauri's `cef-rs`) | Full web compat | V8 | 200-300 MB download at build | Yes | Production-stable, multi-process architecture |

All four render to either pixel buffers (litehtml, Blitz) or GPU shader textures (Servo, CEF). The iced widget handles scrolling and input for the buffer-based backends.

#### Recommendation for email

**litehtml as primary renderer, CEF as optional fallback.** Rationale:

- Email HTML is overwhelmingly table-based layout with inline CSS. litehtml handles this well — it was designed for exactly this class of content. HTTP fetching, image loading, link navigation, and CSS `@import` resolution are already built into the iced integration.
- No JavaScript is needed for email rendering. AMP content is blocked (see section 6). This eliminates Servo and CEF's primary advantage.
- litehtml is the only backend available as a pure crates.io dependency, which avoids git-dep build fragility.
- Blitz is architecturally superior (pure Rust, Stylo for CSS resolution) but is pre-alpha and a git dependency. Worth revisiting when it reaches crates.io and beta stability, likely mid-to-late 2026. Its use of Stylo (MPL 2.0 licensed via `stylo_taffy`) needs license review.
- CEF adds 200-300 MB and multi-process complexity for marginal benefit on email content. Reserve it as an opt-in fallback for users who receive CSS-grid-heavy newsletters that litehtml cannot handle.

#### What other Rust email clients do

- **Himalaya/Pimalaya** (Rust CLI/TUI email): renders `text/plain` only; no HTML rendering.
- **Delta Chat** (Rust core + platform UIs): delegates HTML rendering to the platform webview (Android WebView, iOS WKWebView, desktop Electron). The Rust core provides sanitized HTML; rendering is not done in Rust.
- **Thunderbird**: uses Gecko (Firefox engine) for HTML rendering — not applicable to a pure-Rust stack.

No shipping Rust email client renders HTML email in pure Rust today. This is greenfield work.

#### Rendering architecture for tracking protection

Regardless of backend, the rendering pipeline should be:

1. **Retrieve** raw HTML body from `bodies.db` (zstd-decompress)
2. **Sanitize** (see section 2) — strip dangerous elements, rewrite/block URLs
3. **Inject** tracking-blocking rules (remove `<img>` with remote `src`, strip `<link>` and `@import`, etc.) unless the sender is in the `image_allowlist`
4. **Pass** sanitized HTML to the rendering backend
5. **On user action** ("Load images"), re-render with remote images permitted for that message or persist to allowlist for that sender

This pipeline runs entirely in the core crate before any UI code touches the content. The rendering backend receives pre-sanitized HTML and never makes network requests on its own — all image fetching goes through a controlled local proxy or fetch layer (see section 3).

### 2. HTML sanitization crates

#### ammonia

- **Crate**: [`ammonia`](https://crates.io/crates/ammonia) v4.1.2 (Sep 2025)
- **Maintainer**: Michael Howell / rust-ammonia org
- **Downloads**: ~378K/month, 231 reverse dependencies
- **License**: MIT / Apache-2.0
- **Parser**: html5ever (browser-grade HTML5 parsing, resilient to obfuscation)

Whitelist-based sanitizer. Builds a DOM via html5ever, traverses it replacing disallowed nodes, serializes back. Sanitizes sample text in ~88 microseconds. Configurable: allowed tags, attributes, URL schemes, CSS properties. Can strip all `<img>`, `<link>`, `<style>`, `<script>` tags, and filter `style` attributes to remove `background-image: url(...)` references.

**Strengths for email**: Battle-tested, fast, correct HTML5 parsing. Can be configured to strip all remote resource references (images, stylesheets, fonts) while preserving layout structure (tables, divs, spans with inline color/sizing). The whitelist approach is inherently safe — anything not explicitly allowed is removed.

**Limitations**: Operates on a full DOM (not streaming). For email bodies this is fine (they are small), but it means the entire HTML must be in memory. Does not understand CSS `@import` or `url()` inside `<style>` blocks — those require separate CSS sanitization.

#### lol_html

- **Crate**: [`lol_html`](https://crates.io/crates/lol_html) v2.7.1 (Feb 2026)
- **Maintainer**: Cloudflare
- **License**: BSD-3-Clause

Streaming HTML rewriter with CSS-selector-based API. Designed for Cloudflare Workers to modify HTML on-the-fly with minimal buffering. Does not build a DOM — operates on a token stream, which makes it extremely memory-efficient and fast on large documents.

**Strengths for email**: Can surgically rewrite specific elements using CSS selectors. For example: `img[src^="http"]` to remove remote images, `link[rel="stylesheet"]` to strip external CSS, `a[href]` to rewrite tracking URLs. Streaming means it handles even pathologically large HTML emails without memory pressure.

**Limitations**: Not a sanitizer — it is a rewriter. It does not validate or whitelist; it transforms. Must be used *in addition to* a sanitizer (ammonia), not instead of one. Cannot inspect the full DOM structure (no "remove this element if its computed style makes it 1x1 pixel"). The CSS selector API is powerful but cannot express semantic rules like "remove all invisible images."

#### html5ever + markup5ever

- **Crate**: [`html5ever`](https://crates.io/crates/html5ever) v0.38.0
- **Maintainer**: Servo project
- **Downloads**: ~12M total
- **License**: MIT / Apache-2.0

Low-level browser-grade HTML5 parser. Produces a token stream or can build a tree via `markup5ever`. This is the parser that ammonia uses internally. Direct use only makes sense if you need custom DOM manipulation that ammonia's API cannot express.

#### css-inline

- **Crate**: [`css-inline`](https://crates.io/crates/css-inline) v0.19
- **Maintainer**: Stranger6667
- **License**: MIT

Inlines `<style>` blocks and external stylesheets into `style` attributes on individual elements. Designed for email preparation but equally useful for email *reading*: after inlining, you can remove all `<style>` and `<link>` elements entirely, eliminating CSS-based tracking vectors (`@import url(...)`, `background-image: url(...)` in class-referenced rules). Processes typical emails in hundreds of microseconds.

#### Recommended sanitization pipeline

```
Raw HTML body
  │
  ├─ css-inline: inline all <style> blocks into style="" attributes
  │
  ├─ lol_html (streaming rewrite):
  │    ├─ Strip <link>, <style>, <script>, <iframe>, <object>, <embed>
  │    ├─ Strip <img src="..."> where src is remote (unless allowlisted sender)
  │    ├─ Rewrite style="" to remove url(...) references
  │    ├─ Strip <meta http-equiv="refresh">
  │    └─ Optionally rewrite <a href> through tracking-URL detector
  │
  ├─ ammonia (DOM sanitization):
  │    ├─ Whitelist allowed tags (table layout, text formatting, lists)
  │    ├─ Whitelist allowed attributes (style, class, colspan, rowspan, etc.)
  │    ├─ Remove anything not explicitly allowed
  │    └─ Enforce safe URL schemes (no javascript:, data: with caveats)
  │
  └─ Sanitized HTML → rendering backend
```

Using `lol_html` first for targeted stripping (streaming, fast) followed by `ammonia` as a safety net (whitelist, correct) gives defense-in-depth. If `lol_html` misses something, ammonia catches it. Total overhead: under 1 ms for typical emails.

### 3. Image proxy / rewriting approach

Three strategies for handling remote images when the user chooses to load them:

#### Strategy A: Direct fetch with privacy headers (simplest)

When the user clicks "Load images," fetch each remote image URL using `reqwest` with:
- No cookies
- No referrer header
- Generic `User-Agent` (or none)
- Timeout (5s) and size limit (10 MB per image)
- Cache to local disk keyed by URL hash

**Pros**: Simple, no infrastructure. **Cons**: Sender still sees the request (learns IP, timing). Acceptable for "I trust this sender" scenarios.

#### Strategy B: Prefetch-on-receive (Apple Mail approach)

When a message arrives during sync, immediately fetch all remote images through a privacy proxy or directly, regardless of whether the user opens the message. Cache locally. When the user opens the message, serve from cache.

Apple Mail implements this via a dual-relay proxy to hide the user's IP. Without infrastructure, we can approximate this by fetching on sync (hides *when* the user read it) but the sender still sees the fetch (knows delivery happened). This partially defeats tracking — the sender cannot distinguish "opened" from "received."

**Pros**: Defeats open-time tracking. **Cons**: Bandwidth cost (fetching images for emails you never read), storage cost, does not hide IP without a proxy. Could be offered as an opt-in "enhanced privacy" mode.

#### Strategy C: URL rewriting through local proxy (most control)

Run a lightweight HTTP server on localhost (e.g., `127.0.0.1:PORT`). Rewrite all `<img src="https://remote/...">` to `<img src="http://127.0.0.1:PORT/proxy?url=ENCODED">`. The local proxy:
- Strips tracking query parameters (`utm_*`, `mc_eid`, etc.) using URL parsing (`url` crate)
- Fetches the clean URL with privacy headers
- Caches the response
- Returns the image to the rendering backend

**Pros**: Full control over every request. Can strip tracking params, enforce size limits, detect 1x1 tracking pixels (check response `Content-Length` or decoded image dimensions), and log what was blocked. **Cons**: Adds a localhost HTTP server dependency, more complex architecture.

#### Recommended approach

**Strategy A for MVP, Strategy C as enhancement.** The `image_allowlist` table and `block_remote_images` setting already exist in the core DB. The pipeline is:

1. Default: all remote URLs in sanitized HTML are replaced with a placeholder (broken-image icon or "Images blocked" banner)
2. User clicks "Load images for this message" → re-sanitize with remote images permitted, fetch via Strategy A, cache locally
3. User clicks "Always load from this sender" → add to `image_allowlist`, same as above
4. Future: add Strategy C proxy for tracking-param stripping and 1x1 detection

#### Tracking parameter stripping

The [`tracking-params`](https://docs.rs/tracking-params/) crate removes known tracking query parameters from URLs. The [`url-cleaner`](https://crates.io/crates/url-cleaner) crate provides more comprehensive URL cleaning with configurable rules. Both are small and can be integrated into the sanitization pipeline to clean URLs before fetching.

Known tracking parameters to strip: `utm_source`, `utm_medium`, `utm_campaign`, `utm_content`, `utm_term`, `mc_eid`, `mc_cid`, `fbclid`, `gclid`, `_hsenc`, `_hsmi`, `mkt_tok`, `trk`, `trkCampaign`, `sc_campaign`, `sc_channel`.

### 4. MDN (Message Disposition Notification) suppression

#### How MDNs work

MDNs are defined in [RFC 8098](https://www.rfc-editor.org/rfc/rfc8098.html) (obsoletes RFC 3798). A sender requests a read receipt by including a `Disposition-Notification-To` header in the message. When the recipient's MUA opens the message, it *may* generate an MDN — a new MIME message of type `multipart/report; report-type=disposition-notification` — and send it back to the address in that header.

Key points from the RFC:
- **MDN sending is always optional.** The RFC explicitly states that MUAs may silently ignore MDN requests to preserve user privacy. "Manual sending of MDNs must be the default."
- **MDNs leak information**: reading time, MUA software, OS, IP address. They are a tracking vector comparable to tracking pixels.
- Two sending modes: `MDN-sent-manually` (user explicitly approved) and `MDN-sent-automatically` (MUA configured to auto-send).

#### Header detection with mail-parser

The `mail-parser` crate (Stalwart Labs, the same author as our JMAP server) does **not** have a dedicated `HeaderName` variant for `Disposition-Notification-To`. It falls into the `Other(String)` catch-all. Detection requires:

```rust
// Pseudocode — check for MDN request in parsed message
let has_mdn_request = message.headers().iter().any(|h| {
    matches!(h.name(), HeaderName::Other(name) if name.eq_ignore_ascii_case("disposition-notification-to"))
});
```

This is reliable but means we cannot use typed accessors. The header value is a mailbox address that should be parsed with the same address-parsing logic used for `From`/`To`.

#### Suppression implementation

Suppression is client-side only — we simply never generate or send the MDN message. The implementation:

1. **Parse**: When displaying a message, check for `Disposition-Notification-To` header
2. **Check policy**: Look up the read-receipt policy for this account/sender (see section 5)
3. **Act**:
   - Policy `never`: do nothing (suppress silently)
   - Policy `ask`: show a non-intrusive banner: "Sender requested a read receipt. [Send] [Ignore] [Always ignore from this sender]"
   - Policy `always`: auto-generate and send the MDN
4. **Track**: Set the `$MDNSent` keyword on the message (IMAP/JMAP) or equivalent flag to prevent duplicate MDNs across clients

#### MDN generation

When the user approves sending an MDN, we must generate a `multipart/report` message per RFC 8098 and send it via the appropriate provider. The [`mail-builder`](https://crates.io/crates/mail-builder) crate (also Stalwart Labs) can construct MIME messages, though it has no MDN-specific API — we would construct the disposition notification body part manually. The format is straightforward:

```
Content-Type: multipart/report; report-type=disposition-notification

--boundary
Content-Type: text/plain
[Human-readable receipt text]

--boundary
Content-Type: message/disposition-notification
Reporting-UA: Ratatoskr; 1.0
Final-Recipient: rfc822;user@example.com
Original-Message-ID: <original-msg-id>
Disposition: manual-action/MDN-sent-manually; displayed
--boundary--
```

### 5. Read receipt policies per provider

MDN handling differs significantly across the four providers:

#### Exchange (Graph API)

- **Detection**: The `isReadReceiptRequested` boolean property on the [message resource](https://learn.microsoft.com/en-us/graph/api/resources/message) indicates whether a read receipt was requested. This maps to the `Disposition-Notification-To` header internally.
- **Suppression**: Graph API provides a `SuppressReadReceipt` response object. POST to `/messages/{id}/suppressReadReceipt` to explicitly suppress. Alternatively, simply never call the send-receipt endpoint.
- **Server-side behavior**: Exchange treats read receipts as a client-side function. The server does not auto-send MDNs. OWA and Outlook have their own client-side policies (auto/ask/never), configurable by Exchange admin via transport rules.
- **Our implementation**: Read `isReadReceiptRequested` from the message object during sync. Store in a `mdn_requested` column. Apply the user's policy when the message is opened.

#### Gmail API

- **Detection**: Gmail's API does not expose `Disposition-Notification-To` as a first-class property. Must parse the raw RFC 822 headers from the message payload. The header is accessible via `message.payload.headers[]`.
- **Suppression**: Gmail does not auto-send MDNs. The client is solely responsible. Simply not sending one is sufficient.
- **Workspace read receipts**: Google Workspace (paid) supports read receipts, but this is a Workspace-level feature, not a Gmail API feature. It works via the same `Disposition-Notification-To` header.
- **Our implementation**: Parse the header from raw message data. Same policy engine as other providers.

#### JMAP (RFC 9007)

- **Detection**: [RFC 9007](https://www.rfc-editor.org/rfc/rfc9007.html) defines a proper `MDN` data type with `MDN/parse` and `MDN/send` methods. The `urn:ietf:params:jmap:mdn` capability indicates server support. The `Disposition-Notification-To` header is available in the parsed email.
- **The `$mdnsent` keyword**: JMAP uses the `$mdnsent` keyword (case-sensitive, lowercase) on emails to track whether an MDN has been sent. The client MUST NOT send an MDN if this keyword is already set. After sending, the client MUST set it. The server may return an `mdnAlreadySent` error if the keyword is already present.
- **Suppression**: Simply do not call `MDN/send`. Do not set `$mdnsent` (leave the message without the keyword so the user can change their mind later, or set it to prevent other clients from prompting).
- **Our implementation**: Check for `urn:ietf:params:jmap:mdn` capability. Use `MDN/send` when policy allows. Always check/set `$mdnsent` keyword.

#### IMAP (RFC 3503)

- **Detection**: Parse `Disposition-Notification-To` from the raw message headers (same as Gmail).
- **The `$MDNSent` keyword**: [RFC 3503](https://www.rfc-editor.org/rfc/rfc3503.html) defines the `$MDNSent` IMAP keyword. Before sending an MDN, the client MUST check that this keyword is not already set. After sending, the client MUST set it. The server must support either `$MDNSent` specifically or arbitrary keywords (check `PERMANENTFLAGS`).
- **Suppression**: Do not generate the MDN message. Optionally set `$MDNSent` to prevent other IMAP clients from prompting the user for the same message.
- **MDN sending**: Generate the MDN message per RFC 8098, send it via SMTP (or IMAP APPEND to Sent + SMTP submission). Unlike JMAP, there is no protocol-level "send MDN" command.
- **Our implementation**: Check `PERMANENTFLAGS` for `$MDNSent` or `\*`. Parse the header. Apply policy. When sending, use the existing SMTP submission path and set the IMAP keyword.

#### Policy data model

New DB table for read-receipt policies:

```sql
CREATE TABLE read_receipt_policy (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    scope TEXT NOT NULL,        -- 'global', 'domain:example.com', 'sender:user@example.com'
    policy TEXT NOT NULL,       -- 'ask', 'always', 'never'
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(account_id, scope)
);
INSERT INTO settings (key, value) VALUES ('default_read_receipt_policy', 'ask');
```

Policy resolution: most-specific wins. Check `sender:X` first, then `domain:D`, then account-level default, then global default.

### 6. AMP for Email

#### What it is

AMP for Email allows interactive, dynamic content in emails using a restricted subset of the AMP HTML framework. It uses the `text/x-amp-html` MIME part type alongside the standard `text/html` and `text/plain` alternatives in a `multipart/alternative` message.

#### Security model

AMP emails can make HTTP requests (via `amp-list`, `amp-form`), execute templating logic, and render dynamic content. The spec restricts this:
- No custom JavaScript (only AMP components)
- All network requests must be proxy-able (to prevent IP leakage)
- No authentication on XHR calls (all requests are anonymous)
- All fetchable URLs must be static (inspectable by spam filters)

Despite these restrictions, AMP emails are fundamentally **active content that phones home**. They are antithetical to tracking protection.

#### Our approach: block entirely

Do not render `text/x-amp-html` parts. When a message contains a `multipart/alternative` with a `text/x-amp-html` part:

1. **Prefer** `text/html` (with our sanitization pipeline applied)
2. **Fall back** to `text/plain` if no HTML part exists
3. **Never** render the AMP part

This is what most privacy-focused clients do. Gmail is the only major client that renders AMP content, and even Gmail strips it on forward/reply. No litehtml or Blitz backend can execute AMP anyway (no JavaScript), so this is also a technical necessity.

Detection is straightforward: check the MIME type of each part during body parsing. The `mail-parser` crate parses `multipart/alternative` structures and exposes content types, so filtering out `text/x-amp-html` is a one-line check.

### 7. Link tracking detection

#### The problem

Many email marketing platforms rewrite all links through tracking redirects:
- `https://click.mailchimp.com/track?u=HASH&id=HASH&e=HASH` → actual destination
- `https://links.example.com/redirect?url=ENCODED_DEST&token=TOKEN`
- `https://mandrillapp.com/track/click/XXXXX/destination.com`

The user sees "Click here" but the `href` points to a tracking domain. Clicking reveals their identity, timing, and interest to the sender.

#### Detection approach

Maintain a list of known tracking/redirect domains (similar to ad-blocker domain lists):

```
click.mailchimp.com
links.mcsv.net
mandrillapp.com
track.hubspot.com
links.sendgrid.com
tracking.constantcontact.com
clicks.mlsend.com
links.iterable.com
t.sidekickopen*.com
```

For detected tracking URLs:
1. **Visual indicator**: Show a small shield/warning icon next to the link in the rendered email
2. **Tooltip**: "This link goes through a tracking redirect. The destination appears to be: [decoded URL]"
3. **Optional rewrite**: Attempt to extract the actual destination URL from the tracking URL's query parameters (common patterns: `url=`, `redirect=`, `dest=`, `target=`) and offer to open that directly

This is informational, not blocking. Users in corporate environments receive marketing emails legitimately and need the links to work. The goal is awareness, not prevention.

#### Crate support

- [`url`](https://crates.io/crates/url): parse tracking URLs, extract query parameters
- [`url-cleaner`](https://crates.io/crates/url-cleaner): configurable URL cleaning rules, can strip tracking params
- [`tracking-params`](https://docs.rs/tracking-params/): focused specifically on removing tracking query parameters

The tracking domain list should be a shipped resource file (like ad-blocker filter lists) that can be updated independently of the application binary.

### 8. Comparison with other email clients

| Feature | **Thunderbird** | **Apple Mail** | **Fastmail (web)** | **Outlook (desktop)** | **Ratatoskr (planned)** |
|---------|----------------|---------------|-------------------|----------------------|------------------------|
| Block remote images by default | Yes | Opt-in (Block All Remote Content) | Opt-in (per-account setting) | No (on by default in some configs) | **Yes** |
| Per-sender image allowlist | No (load per-message only) | No | Yes (contacts-based) | Limited (safe senders list) | **Yes** (existing `image_allowlist` table) |
| Privacy proxy for images | No | Yes (Mail Privacy Protection: dual-relay proxy, prefetch on receive) | Yes (server-side proxy, strips IP) | No | **No** (local fetch only; proxy is future work) |
| Tracking pixel detection | No (blocks all or nothing) | No (proxy defeats tracking passively) | No | No | **Planned** (1x1 image detection, tracking domain list) |
| MDN suppression | Yes (ask by default, configurable) | Yes (ask by default) | Yes (server suppresses by default) | Configurable (ask/always/never, admin-controllable) | **Planned** (per-account/per-sender policy) |
| Read receipt policy granularity | Global only | Global only | Global only | Per-account (via Exchange admin) | **Per-sender** (most-specific-wins resolution) |
| AMP blocking | N/A (no AMP support) | N/A | Renders AMP (Fastmail is an AMP sender) | N/A | **Block entirely** |
| Link tracking detection | No | No | No | Safelinks (rewrites through Microsoft proxy, not privacy-focused) | **Planned** (tracking domain list + visual indicator) |
| CSS tracking vector blocking | Yes (blocks external CSS) | Via proxy | Via proxy | No | **Yes** (strip `<link>`, `@import`, inline via `css-inline` then sanitize) |

#### Key differentiators for Ratatoskr

1. **Per-sender read receipt policy** — no mainstream client offers this granularity. Corporate users need "always send to my boss, never send to marketing" rules.
2. **Defense-in-depth sanitization** — the `css-inline` + `lol_html` + `ammonia` pipeline is more thorough than any single-pass approach. Most clients either block images (Thunderbird) or proxy them (Apple Mail) but do not sanitize CSS tracking vectors.
3. **Link tracking transparency** — no client currently tells users when a link goes through a tracking redirect. This is a meaningful privacy UX improvement.
4. **No server-side dependency** — Apple Mail and Fastmail rely on their own proxy infrastructure. Our approach works entirely offline / client-side, which matters for enterprise users on restricted networks.

### 9. Implementation priority

1. **Sanitization pipeline** (`css-inline` + `lol_html` + `ammonia`) — prerequisite for all HTML rendering. Implement in `ratatoskr-core` as a `sanitize_html_body()` function. This is needed regardless of tracking protection, since we must sanitize before passing to litehtml/Blitz.
2. **Remote image blocking** — already implemented in the DB layer (`image_allowlist`, `block_remote_images` setting). Wire into the sanitization pipeline: strip remote `<img src>` unless sender is allowlisted.
3. **MDN suppression** — detect `Disposition-Notification-To` header during message parsing, store `mdn_requested` flag, implement policy table and resolution logic. No UI needed initially (default policy: suppress silently).
4. **Read receipt policy UI** — settings screen for per-account/per-sender policies. Banner in reading pane when MDN is requested.
5. **AMP blocking** — trivial: skip `text/x-amp-html` MIME parts during body selection. Can be done in the existing body-parsing path.
6. **Link tracking detection** — ship a tracking-domain list, integrate URL rewriting into the sanitization pipeline, add visual indicators to the rendered HTML.
7. **Image proxy** (Strategy C) — lowest priority. The MVP works without it. Add when users request IP-hiding or tracking-param stripping.
