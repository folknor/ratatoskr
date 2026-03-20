# TODO

## Inline Image Store Eviction

- [ ] **Wire up user-configurable eviction for `inline_images.db`** — The Rust backend has the building blocks (`prune_to_size()`, `delete_unreferenced()`, `stats()`, `clear()`), but eviction is not yet exposed in the UI.

  **What's missing**:
  1. **Settings UI**: No user-facing control for inline image store size. The 128 MB cap is hardcoded in Rust.
  2. **Scheduled eviction**: No periodic sweep to catch edge cases (e.g., if `MAX_INLINE_STORE_BYTES` is lowered in a future update).

## Iced App

- [x] **Investigate iced ecosystem projects** — Done. See `research/iced-ecosystem-survey.md` (14 repos analyzed), `docs/iced-ecosystem-cross-reference.md` (cross-referenced against all 17 specs), and `## Ecosystem Patterns` sections appended to each doc in `docs/`.

## Cross-Cutting Architecture (from ecosystem survey)

These patterns appeared across 6-8+ specs and should be adopted as foundational infrastructure before feature work builds on top of them. Full rationale in `docs/iced-ecosystem-cross-reference.md`.

- [ ] **Generational load tracking** — Add a `load_generation: u64` counter pattern for all async-load-then-display paths. Increment on every new request (thread selection, search keystroke, scope switch, pinned search click), tag the spawned `Task` with the current generation, discard results in `update()` if the generation has moved on. Prevents stale data from overwriting current state during rapid navigation. Affected: thread detail, search results, sidebar nav state, attachment loading, body store queries, command palette option resolution, status bar sync progress. Reference: bloom (`research/bloom/src/app.rs` line 157).

- [ ] **Component trait for panel isolation** — Define a `Component` trait in `crates/app/` that each major panel implements (`update()`, `view()`, `subscription()`, returning `(Task, ComponentEvent)` tuples). Panels: sidebar, thread list, reading pane, compose, calendar, command palette, status bar, settings. This eliminates the nested `Message` enum problem — each panel owns its own message type and emits typed `ComponentEvent`s that the top-level `App::update()` dispatches. Without this, the `Message` enum will grow unboundedly as features land. Reference: trebuchet (`research/trebuchet/src/app.rs` lines 95-132).

- [ ] **Token-to-Catalog bridge for theming** — Create an `AppTheme` newtype wrapping iced's `Theme` that implements all widget `Catalog` traits (`button::Catalog`, `container::Catalog`, etc.), pulling colors from `theme.palette()` via the existing seed system. This lets widgets use `.class(ButtonClass::Primary)` instead of inline style closures, and ensures all email-specific semantic styles (starred card tint, unread subject color, label dot colors, muted text) flow through one adapter. The existing ~30 style functions in `theme.rs` become methods on `AppTheme`. Reference: iced-plus (`research/iced-plus/iced_plus_theme/src/theme.rs`), shadcn-rs (`research/shadcn-rs/crates/iced-shadcn/src/tokens.rs`).

- [ ] **Vendor iced_drop for drag-and-drop** — Add `research/iced_drop` (623 lines, zero deps beyond iced) as a vendored crate or workspace dependency. Needed by: thread reordering, label drag-to-file, account reordering in settings, compose token DnD between To/Cc/Bcc, group editor member DnD, calendar event dragging, attachment drag zones. The `Operation` trait pattern for widget tree traversal is also useful beyond DnD (finding focused widgets, swapping state between panels). Reference: iced_drop (`research/iced_drop/src/widget/operation/drop.rs`).

- [ ] **Subscription orchestration pattern** — Establish a standard pattern for `Subscription::batch()` combining all background event streams: sync pipeline events, keyboard/hotkey capture, timer ticks (status bar cycling, auto-save, debounce), file system watches, and OS appearance changes. Each subsystem provides a `fn subscription() -> Subscription<Message>` that the top-level `App::subscription()` batches. Use `subscription::channel` with `tokio::select!` for subsystems that multiplex multiple async sources (e.g., sync across 4 providers simultaneously). Reference: pikeru (`research/pikeru/`), rustcast (`research/rustcast/src/app/tile/elm.rs` lines 158-237).

- [ ] **DOM-to-widget pipeline for HTML email rendering** — Evaluate cedilla's frostmark approach as a third option alongside CEF and litehtml: parse sanitized HTML with html5ever, walk the DOM tree with a visitor pattern, emit iced `Element`s (paragraph → `text()`, image → `image()`, link → styled `button`, table → nested `row()`/`column()`). This could handle simple/medium-complexity emails (text, images, links, basic formatting) natively in iced without an external renderer. Complex HTML (CSS-heavy marketing emails) would still need CEF/litehtml fallback. Prototype against a representative sample from the test corpus to determine the complexity threshold. Reference: cedilla/frostmark (`research/cedilla/`), with image caching pattern from `MarkdownPreview` (HashMap<String, image::Handle>).

- [ ] **Patch-based undo/redo for compose editor** — When the compose editor lands, use the `dissimilar` crate for compact diff-based undo history instead of storing full text snapshots. Maintain a ~100-patch circular buffer. This matters for large HTML email drafts where full-snapshot undo would be expensive. Also applicable to any future inline-editing surface (contact notes, calendar event descriptions). Reference: cedilla (`research/cedilla/src/editor.rs`, `EditorState::push_history`).

- [ ] **Config shadow pattern for settings/edit flows** — Any UI that edits persistent state (account settings, preferences, contact editor, calendar event editor) should clone the real state into an `editing_*` shadow on open. The user edits the shadow; commit writes it back, cancel discards it. This prevents partial saves, enables live preview, and makes "has anything changed?" detection trivial (compare shadow to original). Reference: bloom (`research/bloom/src/app.rs` lines 38, 196, 402).

- [ ] **Make sidebar fixed-width (not resizable)** — The sidebar should be a fixed width, not draggable. Remove the sidebar resize divider and any sidebar width persistence from `WindowState`. The sidebar width is a constant in `layout.rs`, not a user preference.

- [ ] **Per-pane minimum resize limits** — PaneGrid currently uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px, contact sidebar below 180px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum. Decide on actual values after visual testing.

- [ ] **Animated toggler widget** — Port libcosmic's slerp-based toggle animation for smooth sliding pill togglers. Current iced built-in toggler snaps instantly. libcosmic's version (`research/libcosmic/src/widget/toggler.rs`) uses `anim::slerp()` with configurable duration (200ms default), interpolating knob position per-frame via `RedrawRequested`. ~150-200 LOC to port.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Keybinding display and edit UI** — Need to redo the Settings/Shortcuts UI. Take a look at https://nyaa.place/blog/libadwaita-1-8/

- [ ] **UI freezes after ~20 minutes with settings open** — App hangs completely with no stdout/stderr. Prime suspect is the `mundy` subscription (`appearance.rs`) holding a D-Bus connection that may drop or block over time. Bisect by disabling subscriptions one-by-one to isolate.

- [ ] **License display/multiline static text row** — Need to be able to click links and make text selectable/copyable in license display widgets. Needs its own base row type.

- [ ] **Restore OS-based theme and 1.0 scale** — `SettingsState::default()` currently hardcodes `theme: "Light"` for development convenience. Revert to `theme: "System"` once UI prototyping is done, and persist user preferences to disk.

- [ ] **Wire up system font detection** — `crates/system-fonts/` is built and working (queries xdg-desktop-portal on Linux, SystemParametersInfo on Windows) but not wired into the app. Two phases:
  1. **UI font**: On startup, call `SystemFonts::detect().await`. If a UI font is found and it's available on the system, apply it via iced's `font::set_defaults` task. Fall back to bundled Inter if detection fails or font isn't installed. This just confirms/overrides the default app font.
  2. **Document font** (separate, later): The detected document font (e.g., TisaPro) should be used for email body text and other long-form content. This requires threading a separate font through the thread detail view and message body widgets. Not straightforward right now — email bodies are HTML processed by the sanitizer pipeline, not iced text widgets, so the document font may only apply once we have native body rendering. Add as a font setting in the settings UI too (let users override the detected system fonts).

- [ ] **Thread list keyboard navigation** — Arrow Up/Down to move selection, PgUp/PgDn to jump by a page, Home/End to jump to first/last. Should scroll the selected thread into view automatically. Enter to open thread, Escape to deselect. Needs an iced keyboard event subscription in the app, gated on the thread list having focus.

- [ ] **Scrollbars must shift layout, not overlay** — When a scrollbar appears (e.g., content grows beyond the viewport), it must push the content inward rather than overlaying existing UI elements. Overlay scrollbars cause text/buttons to be hidden behind the scrollbar track. Ensure all `scrollable` widgets use a mode that reserves space for the scrollbar or accounts for its width in the layout.

- [ ] **Thread list pagination (revisit later)** — Currently loads all threads at once (LIMIT 1000). This is fast with the test dataset (1000 threads renders instantly). We attempted batched lazy loading (200 per page, `on_scroll` trigger, spacer for honest scrollbar) but reverted: (1) `on_scroll` fires on every pixel of scroll movement, causing a full `update()`/`view()` cycle per pixel which made scrolling sluggish; (2) the spacer approach for honest scrollbar sizing made the content area huge, worsening the `on_scroll` overhead; (3) without the spacer the scrollbar thumb jumps when batches load (content height changes suddenly). The DB layer already supports `LIMIT`/`OFFSET` (`db.get_threads` has the params, `count_threads` exists). Revisit when thread counts actually cause problems — likely needs iced-level virtual scrolling (only render visible rows) rather than application-level pagination, since the bottleneck is widget count in the scrollable, not query speed.

## UI Specs Needed

- [ ] **Design Signatures UI** — Signature management lives in Settings. Needs spec for: creating/editing/deleting signatures, per-account default signature assignment, rich text editing (or HTML), signature insertion behavior in compose (new, reply, forward).

- [ ] **Design Emoji Picker** — Shared widget used in compose, calendar event descriptions, contact notes, and anywhere text input supports emoji. Needs spec for: searchable grid, categories/tabs, recent/frequent section, skin tone selection. Separate doc at `docs/emoji-picker/problem-statement.md`.

## Contacts Surface

- [ ] **Implement full contacts crate** — The current `seen-addresses` crate (643 lines, `crates/seen-addresses/`) only tracks sender addresses seen during sync. A proper contacts implementation needs: CardDAV sync (partially started in `core/src/carddav.rs`), contact search/autocomplete, contact detail views, contact groups/labels, merge/dedup, per-provider contact sync (Google People API, Microsoft Graph contacts, LDAP). When this lands, fold `seen-addresses` into the new contacts crate — it's the same domain and shares the same DB tables.

## Code Quality

- [ ] **Decide whether Graph `raw_size = 0` should stay accepted** — Graph still lacks a clean size field for the current query path. Either keep this as an accepted cosmetic limitation or document a better fallback if one exists.

## Microsoft Graph

- [ ] **Ship a default Microsoft OAuth client ID** — Register a multi-tenant Azure AD app ("Accounts in any organizational directory and personal Microsoft accounts"), set as public client (no client secret), configure `http://localhost` redirect URI, request Mail.ReadWrite/Mail.Send/etc. scopes. Ship the client ID as a constant in `oauth.rs`. Then remove the per-account credential UI (the "Update OAuth App" flow in settings that asks users for client_id/client_secret) — users should never see this. Keep the per-account `oauth_client_id` DB column as an optional override for enterprise users who need to use their own tenant-restricted app.

## JMAP

- [ ] **JMAP for Calendars** — `jmap-client` has no calendar support (upstream Issue #3). Blocked until `jmap-client` adds calendar types. Low priority — CalDAV covers calendar sync for now.

- [ ] **Investigate JSContact / JMAP for Contacts support** — Stalwart fully implements JSContact (RFC 9553) and JMAP for Contacts (RFC 9610) with bidirectional vCard conversion. Check whether our JMAP provider crate can use native JMAP Contacts instead of falling back to CardDAV. Audit current `jmap-client` crate for contacts types and determine what (if anything) needs to be added.

## IMAP

- [ ] **QRESYNC VANISHED parsing (Phase 3)** — Send `ENABLE QRESYNC` via raw command, then `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))`. Parse `VANISHED (EARLIER) <uid-set>` untagged responses. Blocked on async-imap CHANGEDSINCE support (Issue #130).
