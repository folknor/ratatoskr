# Pop-Out Windows: Spec vs. Code Discrepancies

Audit date: 2026-05-01

The multi-window foundation is in place: `iced::daemon`, the
`pop_out_windows: HashMap<window::Id, PopOutWindow>` registry, per-window
`view`/`title`/resize/move/close routing, Escape-closes-pop-out, and the
`PopOut(window::Id, PopOutMessage)` dispatch wrapper all work. Message-view,
compose, and calendar pop-outs all render, operate, and survive a session
round-trip. What remains is a mix of unbuilt compose-window features
(drag-drop overlay, attachment compression), the cross-cutting Print
blocker, three stubbed attachment paths, and a handful of fidelity items
called out in the product spec.

This doc is the gating list. When everything below is resolved (or
explicitly cancelled), `docs/pop-out-windows/` can be deleted.

Spec references are to `docs/pop-out-windows/problem-statement.md`
(product surface) and `docs/pop-out-windows/message-view-implementation-spec.md`
(implementation phasing).

---

## High

1. **Compose drag-and-drop two-zone overlay missing.** Problem statement
   §"Attachments" describes a full-window overlay that appears when files
   are dragged over the compose window: the window darkens and two
   semi-transparent zones cover ~94% of the surface side-by-side ("Insert
   inline" left, "Add as attachment" right), with hover highlighting on
   the zone under the cursor. Drop on left = inline insertion, drop on
   right = regular attachment. None of this exists - `pop_out/compose/`
   has no `FilesHovered`/`FilesDropped` handling, no `iced_drop`
   integration, and no overlay widget. The Attach button is the only path
   to add files.

2. **Compose attachment compression not wired.** Problem statement
   §"Attachment Compression" specifies a four-step pipeline: instant
   `squeeze::estimate_file()` on add, running total tracked against the
   sending account's provider limit (Exchange ~7 MB, Outlook/iCloud
   ~15 MB, Gmail/Yahoo ~18 MB), warnings on the attachment bar when the
   total approaches/exceeds the limit, and background compression that
   substitutes the compressed asset at send time while preserving filename.
   Zero `squeeze` calls in `pop_out/compose/` or
   `handlers/pop_out/compose_*`. Attachments are sent as-is.

3. **Compose header is missing a Print button.** Problem statement
   §"Actions" lists `[📎 Attach][🖨 Print][💾 Save][Send]`. The footer
   ships Discard, Attach, Save, and Send - Print is the remaining gap.
   No OS print dialog integration exists anywhere in the project (see
   Medium #11 for the cross-cutting Print blocker).

4. **Recipient contact-pill hover affordance not implemented.** Problem
   statement §"Header Section" (and §"Recipient Fields" for compose):
   "Recipients in the To and Cc fields appear as plain text but become
   contact pills on hover - revealing the inline edit button from the
   contacts spec for quick contact editing". This applies to the message
   view header, the compose recipient summary, and the reading-pane
   message headers. All three currently render plain CSV text with no
   hover interaction, no pill widget, and no `EditContact` wiring from
   the recipient surfaces. Tracked as a `TODO.md` line item.

## Medium

5. **Rendering-mode picker location is unresolved.** Problem statement
   §"Rendering Mode" and the implementation spec Phase 3 both draw the
   picker as a row of four chip-style buttons below the header, above
   the body. `pop_out/message_view.rs:493-502` instead places the four
   modes inside the overflow context menu as radio items. `TODO.md` line
   43 explicitly flags this as unresolved: "the spec currently doesn't
   say clearly where they should go. This needs to be resolved first."
   Either revise the spec to bless the overflow-menu placement or move
   the picker out.

6. **Original HTML mode does not actually fetch remote content.** Problem
   statement §"Rendering Mode": Original HTML "renders the full HTML as
   sent, including remote images and original styles … subject to the
   app's remote-content and tracking-pixel controls". The remote-content
   banner renders (`message_view.rs:688-713`) and `LoadRemoteContent`
   flips `state.remote_content_loaded`, but `block_remote_images` is
   never read in the pop-out path and no actual remote-image fetch is
   wired. Simple HTML and Original HTML render identically. Cross-ref:
   `TODO.md` "Pop out message viewer body rendering toggle buttons".

7. **Source mode synthesizes a pseudo-`.eml` from parsed columns.**
   Problem statement §"Rendering Mode" describes Source as "the raw email
   source (headers + MIME body, monospaced)". `handlers/pop_out/
   message_view.rs:96-139` builds a best-effort reconstruction from
   `from_name/from_address/to_addresses/cc_addresses/subject/date` and
   the body store, with hand-rolled `Content-Type` and no preserved MIME
   boundaries, original headers, transfer encodings, or DKIM/Received
   chain. Faithful Source needs the `raw_source_store` described in
   `TODO.md` "Raw message source store" - a zstd-compressed blob store
   keyed by `(account_id, message_id)` populated during sync, with
   per-provider fetch paths (Gmail `format=raw`, JMAP blob endpoint,
   Graph `/$value`, IMAP `BODY[]`).

8. **Save As is missing `.pdf`.** Problem statement §"Actions" lists
   three formats: `.eml`, `.pdf`, `.txt`. Implementation spec Phase 6
   deliberately defers PDF ("requires rendering the message HTML
   faithfully to a paginated PDF"). `handlers/pop_out/save_as.rs:69-72`
   ships `.eml` and `.txt` only. Either implement HTML-to-paginated-PDF
   rendering or revise the product spec to drop `.pdf`.

9. **Print is a no-op everywhere.** Both surfaces require it: message
    view overflow menu (problem statement §"Actions" overflow item) and
    compose header (problem statement §"Actions"). `pop_out/
    message_view.rs:85` falls into the trailing `Task::none()` arm and
    no compose-side Print exists at all. OS print-dialog integration is
    platform-specific with no iced precedent. Cross-ref: `TODO.md`
    "Pop-out Print". Resolving this also closes High #3.

10. **Attachment Open / Save / Save All in viewer are stubs.** The
    compact attachment cards in `message_view.rs:717-784` render Save /
    Open buttons on hover and a Save All button in the panel header, but
    `handlers/pop_out/message_view.rs:68-79` handles all three with
    `log::info!("not yet implemented")`. The buttons exist and are
    clickable; nothing happens. The reading-pane equivalents in
    `ui/reading_pane.rs:368-376` are also stubs - resolving these
    likely shares an attachment fetch + cache + write path.

11. **Spell check decision deferred.** Problem statement §"Open Questions"
    item 3: "OS-level spell check integration, or custom? Defer to
    implementation." No spell-check infrastructure exists in the editor
    pipeline. Either pick an approach or strike the open question.

## Implemented

### Phase 1 - multi-window architecture

- `iced::application` → `iced::daemon` migration in `main.rs:110`.
- `App.main_window_id` (`app.rs:70`) plus
  `pop_out_windows: HashMap<window::Id, PopOutWindow>` (`app.rs:71`).
- `PopOutWindow` enum has `MessageView`, `Compose`, and `Calendar`
  variants; `PopOutMessage` wraps the per-window message types
  (`pop_out/mod.rs:9-23`).
- `view()` and `title()` route by window ID (`main_view.rs:16-37`,
  `app.rs:350-364`).
- `WindowResized`/`WindowMoved`/`WindowCloseRequested` carry
  `window::Id` and dispatch on whether the ID is `main_window_id`
  (`update.rs:118-154`).
- Main-window close cascades: saves window state + session, syncs all
  dirty compose drafts, closes every pop-out, then `iced::exit()`
  (`handlers/core.rs:770-799`).
- Pop-out close removes from registry; compose pop-outs intercept close
  to show discard confirmation when the body has user content
  (`handlers/core.rs:801-816`).
- Escape closes pop-out windows (and dismisses compose modals first)
  via `handlers/keyboard.rs:18-48`.

### Phase 2 - message view window

- `MessageViewState`, `MessageViewMessage`, `RenderingMode` in
  `pop_out/message_view.rs`.
- Pop-out trigger: `ReadingPaneEvent::OpenMessagePopOut { message_index }`
  emitted by the reading pane (`ui/reading_pane.rs:69,377-392`),
  dispatched by `handlers/core.rs:217-219` to `open_message_view_window`
  (`handlers/pop_out/window_lifecycle.rs:16-49`).
- Sender avatar with BIMI cache fallback to initials, From + email, To,
  Cc, Subject + date in the header (`message_view.rs:268-378`).
- Async body load: tries `BodyStoreState` first, falls back to DB
  snippet (`handlers/pop_out/message_view.rs:142-191`,
  `db/threads.rs:198-205`).
- Async attachment load via `Db::load_message_attachments`
  (`db/threads.rs:208-226`).
- Per-window `GenerationToken<PopOut>` discards stale loads
  (`pop_out/message_view.rs:130-217`,
  `handlers/pop_out/message_view.rs:14-45`).
- Body card scrolls; header pinned; attachments panel pinned at bottom.

### Phase 3 - rendering modes

- Plain text, Simple HTML, Original HTML (via `html_render`), Source
  (synthesized) all render (`message_view.rs:579-637`).
- System-wide default rendering mode persisted via
  `Settings.default_rendering_mode`; new pop-outs and session-restored
  pop-outs both honor it
  (`handlers/pop_out/window_lifecycle.rs:32`,
  `handlers/pop_out/session.rs:69`).
- Monospace font helper at `font.rs:76-77` used by Source mode.
- Remote-content banner appears in Original HTML mode when content is
  blocked (`message_view.rs:688-713`).
- Source mode lazy-synthesizes `raw_source` on first switch
  (`handlers/pop_out/message_view.rs:201-212`).
- Per-window mode override is transient (not persisted in
  `MessageViewSessionEntry`).

### Phase 4 - action buttons

- Reply / Reply All / Forward in the header row open a compose pop-out
  pre-filled from the message view's headers and body
  (`handlers/pop_out/dispatcher.rs:27-42`,
  `handlers/pop_out/window_lifecycle.rs:126-161`).
- Overflow menu with Archive / Delete / Print / Save As, plus the
  rendering-mode radio group, anchored via `AnchoredOverlay`
  (`message_view.rs:420-517`).
- Archive and Delete dispatch through the action service
  (`MailActionIntent::Archive` / `Trash`) using the pop-out's captured
  `source_selection` so the action resolves against its origin context,
  not whatever the main window is showing later
  (`handlers/pop_out/dispatcher.rs:55-71`,
  `handlers/pop_out/message_view.rs:217-243`).
- Compose deduplication: opening a reply for a thread that already has
  an open compose window focuses the existing window via
  `iced::window::gain_focus` (`handlers/pop_out/window_lifecycle.rs:66-72`).
- Auto-select shared-mailbox identity when replying from shared-mailbox
  scope, gated on `may_submit` rights
  (`handlers/pop_out/window_lifecycle.rs:76-87`).

### Phase 5 - session restore

- `SessionState` with `MessageViewSessionEntry`, `ComposeSessionEntry`,
  and `Option<CalendarSessionEntry>` serialized to `session.json`;
  falls back to legacy `window.json` (`pop_out/session.rs`).
- Save on main-window close; restore at boot reopens message-view,
  compose, and calendar pop-out windows
  (`handlers/pop_out/session.rs`,
  `handlers/core.rs:770-799`,
  `app.rs:222,302,346`).
- Message view: re-dispatches body/attachment loads.
  `BodyLoaded(Err)` sets `error_banner` so a deleted-message restore
  surfaces "This message is no longer available"
  (`handlers/pop_out/message_view.rs:24-32`).
- Compose: persists `draft_id` only; on restore, opens the window with
  saved geometry and async-loads the draft from `local_drafts` via
  `Message::RestoredComposeLoaded`. If the draft row is gone the
  window closes itself.
- Calendar: `PopOutWindow::Calendar(CalendarPopOutGeometry)` carries
  width/height/x/y, populated from `WindowResized` / `WindowMoved`
  and persisted via `CalendarSessionEntry`. View and date come from
  the existing `CalendarState` on `App`.

### Phase 6 - Save As (partial; see Medium #9)

- `rfd::AsyncFileDialog` with format filters for `.eml` and `.txt`
  (`handlers/pop_out/save_as.rs:66-93`).
- `.eml` writes raw source from `Db::load_raw_source` (currently
  synthesized; see Medium #8).
- `.txt` writes plain-text body from `Db::load_message_body`.
- Subject sanitized to a safe filename
  (`handlers/pop_out/save_as.rs:38-55`).

### Compose pop-out (beyond the message-view-implementation-spec)

- `ComposeState` / `ComposeMessage` / `ComposeMode` in
  `pop_out/compose/`.
- Open paths: `Message::Compose` → `ComposeMode::New`; reading-pane
  Reply/Reply All/Forward; message-view Reply/Reply All/Forward;
  selecting a local draft from the thread list (`update.rs:157,
  408-417`, `handlers/pop_out/window_lifecycle.rs:126-161`,
  `handlers/core.rs:826-839`).
- From-account picker dropdown with display name, email, account name,
  selection highlight, anchored overlay dismissal
  (`pop_out/compose/view.rs:175-336`).
- Cc/Bcc auto-shown when prefilled (reply-all) and revealed via header
  buttons that disappear once the field is open
  (`pop_out/compose/view.rs:194-218`).
- Subject input with appropriate Re:/Fwd: prefix.
- Token-input recipient fields with autocomplete (DB-backed contact
  search, generation-guarded), context menus (Cut / Copy / Paste /
  Delete / Move-to / Expand-group), drag-and-drop (intra-window),
  bulk-paste banner with save-as-group flow, and Bcc nudge banner for
  groups (`pop_out/compose/view.rs:338-432`,
  `handlers/pop_out/dispatcher.rs:103-339`).
- Inline validation: "Add at least one recipient" surfaces under the
  recipient cluster on send attempt with empty To/Cc/Bcc
  (`pop_out/compose/view.rs:114-126`).
- Rich text editor body (`rte` crate) with per-action `BodyChanged`
  dispatch (`pop_out/compose/view.rs:458-485`).
- Formatting toolbar (Bold / Italic / Underline / Strikethrough / List
  / Link / emoji picker); link dialog overlay; emoji picker anchored
  via `AnchoredOverlay` and inserts via `EditAction::InsertText`.
- Auto-resolved signature on open and on From-account change, inserted
  with a tracked separator index for reply quote placement
  (`handlers/pop_out/compose_signature.rs`).
- Reply mode pre-quotes the original body in a `<blockquote>` with
  attribution; signature placed between content and quoted text on
  reply, at the bottom on new-compose
  (`pop_out/compose/state.rs:281-297`).
- Attach button via `rfd::AsyncFileDialog` with file-content read into
  `ComposeAttachment { name, mime_type, data: Arc<Vec<u8>> }`
  (`handlers/pop_out/dispatcher.rs:341-380`).
- Attachment compression NOT wired (see High #2).
- Auto-save: 30s `ComposeDraftTick` writes dirty drafts to
  `local_drafts`; manual `Save` button in the footer triggers the
  same path on demand; main-window close flushes synchronously before
  destroying compose windows (`handlers/pop_out/compose_draft.rs`,
  `subscription.rs:74-79`).
- Discard confirmation modal when closing a compose window with user
  content (`handlers/core.rs:801-810`,
  `pop_out/compose/view.rs:74-100`).
- Send pipeline routes through the action service with `SendCompleted`
  closing the window on success (`handlers/pop_out/compose_send.rs`,
  `update.rs:211-214`).
- Local drafts in the thread list open in a compose window with all
  state restored from `DbLocalDraft` (`pop_out/compose/state.rs:157-228`).

### Calendar pop-out

- `PopOutWindow::Calendar(CalendarPopOutGeometry)` variant; calendar
  mode-toggle focuses the pop-out instead of switching the main
  window when one exists (`handlers/calendar.rs:355-369`,
  `handlers/core.rs:300-316`, `update.rs:317-362`).
- Calendar UI renders in the pop-out via the same
  `ui::calendar::calendar_layout` used in the main window
  (`main_view.rs:31-33`).
- Geometry tracked from `WindowResized` / `WindowMoved` and persisted
  via `CalendarSessionEntry`; restored at boot.
