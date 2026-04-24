# Pop-Out Windows: Problem Statement

## Overview

Pop-out windows serve two purposes: viewing a message standalone and composing email. They are free-floating windows independent of the main app window, essential for multi-monitor workflows - reference one message while composing another, keep instructions visible while working, draft a long email without losing your place in the inbox.

This document covers both message viewing and compose pop-out windows. The inline composer (reply/forward within the reading pane) is spec'd in `docs/main-layout/problem-statement.md` and is not covered here.

## Window Rules

From the calendar spec's window limits:

- **Multiple pop-out windows allowed** - any number of message view and compose windows can be open simultaneously
- Each pop-out is its own window with its own position and size
- Pop-outs are not full app instances - they share the same process, database connections, and state
- The mail window is the true main window. Closing it closes everything - calendar pop-out, compose windows, message view windows. The app exits.
- Closing the calendar pop-out or any message/compose window only closes that window. No cascade.
- **Session restore:** On launch, the app restores the full window state from the previous session - main window (position, size, mode, scroll positions, selections), calendar pop-out (if it was open, with position, size, view, date), all message view windows (position, size, which message), and all compose windows (position, size, draft state). The user picks up exactly where they left off. Restoration is best-effort - if a message was deleted, a draft was already sent from another device, or an attachment path no longer exists, the window opens in a degraded state (error banner for missing messages, read-only "already sent" notice for sent drafts, missing-file indicator on attachments) rather than silently failing or refusing to open.

### State Ownership

Pop-out windows share process and data but each window owns its own local state. The boundaries:

**Shared state** (canonical, lives in core/db): message content, thread metadata, draft records, contact data, account configuration, attachment files, label state. Changes propagate to all windows - archiving a thread from one window updates it everywhere.

**Window-local state** (per-window, not persisted beyond session restore): editor selection and cursor position, scroll position, rendering mode override, unsaved compose edits (between auto-saves), drag state, window geometry, confirmation dialog state, pending style in the editor.

**Compose ownership rule**: the draft record in the database is the canonical state. The compose window's in-memory state is a working copy that auto-saves back periodically. If the same draft is opened in two windows (e.g., session restore race), the second window should detect the conflict and offer to close or take ownership rather than silently forking the draft.

## Message View Window

Opened by double-clicking a message card in the conversation view.

### Layout

A single-panel window showing one message with full content.

```
┌─ Re: Sprint Planning - alice@corp.com ───────── ─ □ ✕ ┐
│                                                        │
│ From: Alice Smith              [↩] [↩All] [→] [⋮]     │
│       alice@corp.com                                   │
│ To: Bob Jones, Charlie                                 │
│                                                        │
│ Re: Sprint Planning       Wed, Mar 19, 2026 10:42 AM   │
│                                                        │
│ ──────────────────────────────────────────────────────  │
│                                                        │
│ Hey Bob,                                               │
│                                                        │
│ Here's the updated roadmap. Key changes:               │
│ - Moved calendar to Q2                                 │
│ - Added contacts as a dependency                       │
│                                                        │
│ Let me know if the timeline works.                     │
│                                                        │
│ Alice                                                  │
│                                                        │
├─ Attachments (1) ──────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────┐  │
│  │ 📄 roadmap.pdf                                  │  │
│  │ PDF · 2.1 MB · Mar 19 from Alice Smith          │  │
│  └─────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

### Header Section

- **From** - sender display name on the first line, email address on the second line. Action buttons (reply, forward, star, etc.) are right-aligned on the same row as the display name.
- **To** - recipients, display names only (no email addresses)
- **Cc** - display names only (if present)
- **Subject** - displayed below the header fields in a larger font, immediately before the message body
- **Date** - right-aligned on the same line as the subject, baseline-aligned but in a smaller font
- **Attachments** - displayed at the bottom of the window as part of the chrome, using the same card-style rendering as the main reading pane's attachment list. No deduplication or versioning (single message, so no duplicates to resolve).

Recipients in the To and Cc fields appear as plain text but become contact pills on hover - revealing the inline edit button from the contacts spec for quick contact editing. The From field always shows both name and email since it's the sender. All other recipients show display names only (resolved from contacts where available, falling back to the name from the email header, falling back to the email address if no name exists).

The window title bar shows the message subject and sender email.

### Message Body

Full rendered message body. Same rendering pipeline as the main reading pane (HTML sanitization, inline images, etc.).

### Actions

Action buttons are in the top-right corner of the header area, on the same row as the sender name:

**Primary buttons (always visible):**

- **Reply** (↩) - opens a new compose window pre-filled for reply
- **Reply All** (↩All) - opens a new compose window pre-filled for reply-all
- **Forward** (→) - opens a new compose window pre-filled for forward

**Overflow menu (⋮):**

- **Archive** - archives the thread
- **Delete** - moves to trash
- **Print** - prints the message via the OS print dialog
- **Save As** - saves the message to disk via file picker. Formats: `.eml` (RFC 5322, full message with headers and MIME), `.pdf` (rendered), `.txt` (plain text body only). Note: `.eml` and `.txt` are straightforward serialization. `.pdf` is substantially harder - it requires rendering the message HTML faithfully to a paginated PDF, which is a separate rendering pipeline from the screen display. PDF export should be treated as a later-phase feature that does not block the initial pop-out implementation.

Thread-level actions (star, labels) are not shown - the pop-out is a single-message viewing surface.

**Escape** closes the message view window.

### Rendering Mode

A toggle in the header area (alongside the action buttons, or below them) lets the user switch between four rendering modes for the message body:

- **Plain Text** - strips all formatting, shows the `text/plain` part
- **Simple HTML** - basic formatting (bold, italic, lists, links) but strips remote content, heavy styling, and scripts. This is the sanitized output.
- **Original HTML** - renders the full HTML as sent, including remote images and original styles. This mode is subject to the app's remote-content and tracking-pixel controls (see `docs/roadmap/tracking-blocking.md`). If the user has remote content blocked globally, switching to Original HTML shows a banner offering to load remote content for this message rather than silently fetching it. This is a content-trust decision, not just a visual toggle.
- **Source** - shows the raw email source (headers + MIME body, monospaced)

The default mode is a system-wide setting in Settings. The per-message toggle overrides it for that window only (not persisted).

All actions are also available via keyboard shortcuts and the command palette. No inline composer in pop-out message windows (established in `docs/main-layout/problem-statement.md` § Open Questions #4).

## Compose Window

Opened by:

- Popping out the inline composer (pop-out button on the inline reply/forward)
- Clicking Reply/Reply All/Forward in a message view pop-out
- Command palette Compose command (`c`)
- The 📅→email flow from calendar (composing an email about an event)

### Layout

```
┌─ New Message ──────────────────────────────────── ─ □ ✕ ┐
│                                                          │
│ From: [Alice Smith <alice@corp.com> ▾] [Cc][Bcc]  [📎][🖨][💾] [Send] │
│ To:   [Bob Jones] [charlie@corp.com] [         ]         │
│ Cc:   [                                        ]         │
│ Bcc:  [                                        ]         │
│ Subject: [Re: Sprint Planning                  ]         │
│                                                          │
│ ─── Formatting Toolbar ──────────────────────────        │
│ B  I  U  S  │ • ─ 1. │ "" │ 🔗 │ 😀                    │
│                                                          │
│ ──────────────────────────────────────────────────        │
│                                                    │
│ Hi Alice,                                          │
│                                                    │
│ Looks good. One question about the calendar        │
│ timeline - can we pull it into late Q1?            │
│                                                    │
│ Bob                                                │
│                                                    │
│ ─── signature ──────────────────────────────────   │
│ Bob Jones                                          │
│ Engineering Lead · Corp Inc                        │
│                                                    │
│ ── On Mar 19, Alice Smith wrote: ───────────────   │
│ > Here's the updated roadmap. Key changes:         │
│ > - Moved calendar to Q2                           │
│ > - Added contacts as a dependency                 │
│                                                    │
├────────────────────────────────────────────────────┤
│                              [Discard]      [Send] │
└────────────────────────────────────────────────────┘
```

### Recipient Fields

The To, Cc, and Bcc fields use the autocomplete system from `docs/contacts/problem-statement.md` - identical behavior (token input, paste handling, group tokens, context menus, drag between fields, Bcc suggestion for groups, group creation suggestion for bulk paste).

**From field:** An account selector dropdown. Each entry shows the display name and email on the left (`Alice Smith <alice@corp.com>`) and the account name on the right in a secondary font (`Work Account`). Pre-filled based on the resolution order from `docs/sidebar/problem-statement.md` § Default sender account for compose (explicit selection → thread context → last manually selected → current scope → first account).

**Cc and Bcc:** Hidden by default. Two buttons (`Cc`, `Bcc`) on the right side of the From row each reveal their respective field and then disappear (no reason to toggle back). If the compose is pre-filled with Cc or Bcc recipients (e.g., reply-all), those fields are shown automatically and the corresponding buttons are already gone.

### Subject Line

A plain text field. Pre-filled with "Re: ..." or "Fwd: ..." for replies and forwards.

### Formatting Toolbar

A horizontal row of formatting buttons above the compose body. Always visible, not toggled. The exact set of formatting options is TBD during implementation, but expected capabilities include: bold, italic, underline, strikethrough, lists (bulleted/numbered), blockquote, links, horizontal rule.

An **emoji picker** button is required - opens the shared emoji picker (see `docs/emoji-picker/problem-statement.md`) for insertion at the cursor.

The compose body is a rich text editor. The internal format is HTML - what gets sent is HTML email. Plain text fallback is auto-generated from the HTML for the `text/plain` multipart alternative. See `docs/editor/architecture.md` for the editor's document model, widget strategy, and implementation phases.

### Signature

The active signature is inserted automatically below the compose body, separated by a visual divider. Which signature is used depends on the From account's default signature setting (configured in Settings - see `TODO.md`).

Changing the From account updates the signature if the new account has a different default signature.

### Quoted Content

For replies and forwards, the original message is included below the signature as quoted content, prefixed with an attribution line ("On Mar 19, Alice Smith wrote:"). The quoted content is editable - users can trim or annotate it.

For forwards, the original message's attachments are included (the user can remove them before sending).

### Attachments

Files are attached via:

- The 📎 button in the header (opens a file picker - adds as attachment)
- Paste (for images from clipboard - inserts inline)
- **Drag and drop** - when files are dragged over the compose window, a full-window overlay appears. The entire window darkens, and two semi-transparent colored drop zones appear side by side, covering ~94% of the window (the rest is margin). The zone under the cursor gets a hover highlight.

```
┌──────────────────┬─────────────────┐
│                  │                 │
│  Insert inline   │ Add as          │
│                  │ attachment      │
│                  │                 │
└──────────────────┴─────────────────┘
```

Dropping on the left zone inserts the file inline in the message body (images are rendered inline, other files become inline icons). Dropping on the right zone adds the file as a regular attachment.

Attached files appear at the bottom of the window as part of the chrome (same position as the viewer window's attachment list):

```
│ Attachments: [📎 roadmap.pdf ✕] [📎 screenshot.png ✕] │
```

Each attachment shows the filename, size, and a remove button (✕). Double-clicking opens the file with the OS default handler.

#### Attachment Compression

Attachments are transparently compressed via the `squeeze` crate before sending. When a file is added:

1. **Instant estimate** - `squeeze::estimate_file()` runs immediately (header-only read, sub-millisecond for images/archives) to get a conservative size prediction.
2. **Running total** - the compose window tracks the estimated total attachment size against the sending account's provider limit (Exchange ~7 MB, Outlook/iCloud ~15 MB, Gmail/Yahoo ~18 MB).
3. **Warnings** - if the running total approaches or exceeds the limit, a warning is shown on the attachment bar. If a single file's non-compressible floor exceeds the limit, a specific warning explains it can never fit.
4. **Background compression** - files that can benefit from compression are compressed in the background. The attachment size display updates when compression completes, showing only the compressed size.

Compression is transparent - the user doesn't need to configure it. The attachment they see is the original; the compressed version is substituted at send time. Filenames are preserved (the compressed file keeps the original name). For images, compression is lossy (mozjpeg/oxipng) - the visual appearance may differ slightly from the original, but the difference is sub-perceptual at squeeze's quality targets. If an image is inserted inline in the body, the inline preview shows the original; the sent version uses the compressed asset. This is an acceptable divergence because the quality difference is imperceptible in practice.

### Actions

Action buttons in the top-right of the header area, on the same row as the From field:

- **Attach** (📎) - opens a file picker to attach files
- **Print** (🖨) - prints the composed message via the OS print dialog
- **Save** (💾) - saves the draft immediately
- **Send** - sends the email and closes the window. Visually distinct (primary button).

**Discard:** Closing the window (✕ button, Ctrl+W, or other window-close shortcuts) with unsaved content (beyond signature and quoted text) prompts for confirmation. If discarded, the draft is deleted. Escape does not close the compose window.

### Drafts

Compose windows auto-save drafts periodically (every ~30 seconds or on significant changes). Drafts are saved to the drafts folder of the From account. If the user closes and confirms discard, the draft is deleted. If the app crashes, the draft survives in the drafts folder.

Drafts are visible in the thread list when viewing the Drafts folder. Clicking a draft opens it in a compose window with all state restored (recipients, subject, body, attachments).

## Window Sizing

- **Message view windows** default to a reasonable size (~800x600) and remember their last size/position per-session
- **Compose windows** default to a similar size and also remember size/position
- Both are freely resizable with sensible minimums (no collapsing below usability)

## Open Questions

1. ~~**Cc/Bcc visibility**~~ **Resolved.** Hidden by default, toggled via buttons on the From row. Auto-shown when pre-filled.

2. ~~**Rich text editor implementation**~~ **Resolved.** Custom WYSIWYG widget built from scratch using iced's `Paragraph::with_spans` for per-block rendering and a block-tree document model with inline styled runs. No existing iced rich text editor was suitable as a base. Full architecture in `docs/editor/architecture.md`.

3. **Spell check** - OS-level spell check integration, or custom? Defer to implementation.

## Ecosystem Patterns

Patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) that apply to pop-out windows.

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Session restore (positions, sizes) | shadcn-rs `auto_save_id` + rustcast TOML config | `SessionState` struct (serde) with `Vec<WindowState>`; shadcn-rs's auto-save-by-ID concept |
| Rich text compose (undo) | Slate operation invertibility + ProseMirror step mapping | Operation-based undo with `EditOp::invert()` and `PosMap` cursor mapping (see `docs/editor/architecture.md`) |
| Rich text compose (editor) | Custom architecture (ProseMirror + Slate + fleather hybrid) | Block-tree document model with `Paragraph::with_spans` rendering, built from scratch (see `docs/editor/architecture.md`) |
| HTML rendering in message view | cedilla/frostmark | DOM-to-widget pipeline for sanitized HTML |
| DnD attachments (inline vs attachment zones) | iced_drop + shadcn-rs file-drop-zone + bloom clipboard | iced_drop for two-zone overlay; iced's native `FilesHovered`/`FilesDropped` for OS drops; bloom's clipboard fallback |
| Contact autocomplete | shadcn-rs command palette + raffi query routing + pikeru MouseArea | Searchable dropdown; enum dispatch for contacts/groups/recent; right-click on pills |
| Rendering mode toggle | trebuchet Component + bloom config shadow | Mode as Component; system default in config, per-window override in local state |
| Keyboard shortcuts per window type | cedilla key bindings + feu raw keyboard + trebuchet Component | Per-window-type `handle_key_event()`; subscription for Escape capture |
| Auto-save drafts | pikeru subscriptions + cedilla undo + bloom generational | `iced::time::every(30s)` subscription; undo history as change detector |

### Gaps

- **Multi-window management**: No surveyed project uses iced multi-window. Window lifecycle, per-window message routing, and cascade-on-main-close are entirely custom. This is the largest gap for this spec.
- **WYSIWYG HTML compose**: Confirmed as unsolved in the iced ecosystem. A custom editor architecture has been designed - see `docs/editor/architecture.md`.
- **Token/pill input widget**: No surveyed project implements chip/tag input fields. Must be built as a custom `advanced::Widget`.
- **OS print dialog integration**: Platform-specific code needed with no iced precedent.
- **PDF export from rendered email**: cedilla uses server-side Gotenberg; a local solution is needed.
