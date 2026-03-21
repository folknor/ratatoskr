# Contacts Feature: Spec vs Implementation Discrepancies

Audit date: 2026-03-21 (updated). Covers `problem-statement.md`, `import-spec.md`, and `autocomplete-implementation-spec.md`.

---

## What matches the spec

### Token input widget (`crates/app/src/ui/token_input.rs`)

- **Custom `advanced::Widget`** implementation as specified. File location matches spec (`crates/app/src/ui/token_input.rs`).
- **Data types match**: `Token`, `TokenId`, `RecipientField`, `TokenInputValue` all present with the specified fields (`id`, `email`, `label`, `is_group`, `group_id`, `member_count`). `TokenInputValue` has `tokens`, `text`, `next_id` as specified.
- **Wrapping flow layout** implemented correctly: tokens laid out left-to-right with row wrapping, text input placed after tokens, wraps to new row if remaining space < `TOKEN_TEXT_MIN_WIDTH`.
- **Layout constants** all present in `crates/app/src/ui/layout.rs`: `TOKEN_HEIGHT` (24), `TOKEN_RADIUS`, `PAD_TOKEN`, `TOKEN_SPACING`, `TOKEN_ROW_SPACING`, `PAD_TOKEN_INPUT`, `TOKEN_TEXT_MIN_WIDTH` (120), `TOKEN_GROUP_ICON_SIZE`, `AUTOCOMPLETE_MAX_HEIGHT` (300), `AUTOCOMPLETE_ROW_HEIGHT` (32).
- **Keyboard state machine** implemented: backspace-at-start selects last token, backspace-with-selected removes it, comma/semicolon always tokenize, space tokenizes if text contains `@`, Enter/Tab tokenize, Escape blurs.
- **Paste handling** implemented with RFC 5322 parsing via `crates/app/src/ui/token_input_parse.rs`: handles `Name <email>`, `"Name" <email>`, bare email formats with display name extraction. Dedup within paste and against existing tokens. Invalid addresses silently dropped.
- **Token chip drawing**: background quad with rounded corners, selected/hovered states with palette colors, label text rendering.
- **Focus management**: click on token selects + focuses, click in field focuses, click outside blurs.
- **Email validation**: `is_plausible_email()` helper present with basic `@` + `.` check.
- **Arrow key navigation**: Left/Right arrow navigates between tokens. Left at text position 0 selects last token. Right from last token deselects and focuses text.
- **Right-click context menu**: `TokenContextMenu(TokenId, Point)` message emitted on `mouse::Button::Right` click on tokens.
- **Group token visual distinction**: People icon (`icon::users()` glyph) prepended to group tokens. Member count available on Token struct.
- **Text width uses `chars().count()`** instead of `label.len()` byte count, correct for non-ASCII.
- **Bulk paste banner**: 10+ pasted addresses shows a dismissible suggestion banner.
- **Delete key**: Removes selected token (via `keyboard::key::Named::Delete`).

### Autocomplete dropdown (Phase 2)

- **`AutocompleteState`** in `ComposeState` with `active_field`, `query`, `results`, `highlighted`, `search_generation`.
- **`search_contacts_for_autocomplete`** wired to compose via `handlers/contacts.rs` dispatch. Searches contacts, seen addresses, AND contact groups.
- **Dropdown rendering** below focused field with highlighted row, mouse click to select.
- **Generation counter** for stale result discard.
- **Debounced search** via `Task::perform` with the generation pattern.
- **`ComposeMessage::AutocompleteResults/Select/Navigate/Dismiss`** variants implemented.
- **Ranking uses recency** (`last_contacted_at DESC`, `last_seen_at DESC`) not frequency.

### Contact search types

- `ContactMatch` type in `crates/app/src/db/contacts.rs` for autocomplete search, including `is_group`, `group_id`, `member_count` fields.

### Compose integration (`crates/app/src/pop_out/compose.rs`)

- To/Cc/Bcc fields each use `TokenInputValue` with per-field selected token state.
- `ComposeMessage` has `ToTokenInput`, `CcTokenInput`, `BccTokenInput` variants.
- `handle_token_input_msg` dispatches all `TokenInputMessage` variants including new ones.
- Reply/ReplyAll/Forward modes populate tokens correctly.
- Cc/Bcc toggle buttons with show/hide behavior.

### Backend data layer

- `crates/seen-addresses/`: `AddressObservation` with direction scoring (`SentTo`, `SentCc`, `ReceivedFrom`, `ReceivedCc`), `SeenAddressMatch` with score field.
- `crates/core/src/db/queries.rs`: `search_contacts()` with FTS5 + LIKE fallback, contacts/seen-addresses union with deduplication and source priority ranking.
- `crates/core/src/db/queries_extra/contact_groups.rs`: Full CRUD for contact groups, search, recursive group expansion with cycle detection.
- `crates/core/src/db/queries_extra/contacts.rs`: Contact CRUD, stats, same-domain contacts, recent threads, attachments from contact, avatar updates.
- CardDAV support: `crates/core/src/carddav/` with `parse.rs`, `sync.rs`, `client.rs`.
- Contact photos: `crates/core/src/contact_photos.rs`.

### Contact management UI (Settings)

- Contact and group management lives in Settings (as specified).
- Contact cards show: display name, email, email2, phone, company, notes, groups, account color.
- Group cards show: name, member count, created/updated dates.
- Filter inputs for both contacts and groups.
- Slide-in editor overlay for both contacts and groups.
- New Contact / New Group buttons.
- **Delete confirmation** for both contacts and groups (two-step: Delete -> Confirm delete / Cancel).
- **Account selector** dropdown on contact creation/edit (lists connected accounts + "Local").
- **N+1 group membership query replaced** with single JOIN query using `GROUP_CONCAT`.

---

## Divergences from spec

### Token input widget

1. **Missing `DragStarted`, `TokenDropped`, `MoveToken` messages**: The spec defines drag-and-drop messages for moving tokens between fields. Not implemented (requires `iced_drop` integration).

2. **No `DragState`**: The spec defines `DragState { token_id, origin, current }` in `TokenInputState`. No drag state tracking.

3. **Simplified `TokenInputState`**: Spec has `cursor_position: usize` and `drag: Option<DragState>`. Implementation has `token_bounds` and `is_focused`. Token selection is managed externally.

4. **Widget constructor signature differs**: Spec defines `token_input(field, tokens, text, placeholder, on_message)` with `RecipientField` parameter. Implementation is `token_input_field(tokens, text, placeholder, selected_token, on_message)` -- no `field` parameter, adds `selected_token` parameter instead.

5. **Text measurement**: Spec calls for `renderer.measure()` for precise token widths. Implementation uses a character-width heuristic (`chars().count() * TEXT_MD * 0.54`). Correct for multi-byte characters but not pixel-precise.

### Autocomplete dropdown

6. **`ContactSearchResult` types in app crate instead of core**: Spec says `crates/core/src/contacts/search.rs`. Types are in `crates/app/src/db/contacts.rs` as `ContactMatch`. Violates crate boundary but functional.

7. **No `search_contacts_unified` in core**: The app-layer search function handles contacts + seen addresses + groups directly rather than delegating to a core unified search function.

8. **Keyboard interception for dropdown not implemented**: Spec says Up/Down/Enter/Tab should be intercepted at compose level when dropdown is visible. Currently these keys pass through to the token input widget regardless.

### Contact management UI

9. **No distinction between local and synced save behavior**: Spec says local contacts save immediately on edit, synced contacts use explicit Save button. Implementation always uses an explicit Save button regardless of source.

10. **No provider write-back on save**: Spec says synced contact edits are pushed to the provider API. Implementation saves locally only.

11. **Inline contact editing popover**: Spec describes a popover on reading pane sender/recipient pills for quick contact editing. Not implemented.

### Contact import

12. **Entire import feature missing**: The spec defines a `crates/contact-import/` crate for CSV/XLSX/vCard import with encoding detection, column mapping, preview, and an import wizard UI. No such crate exists. No import UI in settings.

### GAL caching

13. **GAL/directory caching not implemented**: Spec describes pre-fetching organization directory at startup with polling refresh. No GAL cache implementation found in the codebase.

---

## Cross-cutting concerns

### a. Generational load tracking

**Used in autocomplete.** `search_generation: u64` in `AutocompleteState` discards stale search results, matching the pattern used elsewhere (`nav_generation`, `thread_generation`).

### b. Component trait

**Contacts UI is not componentized.** The settings panel (`Settings`) implements `Component`, and contacts management lives within it as part of the People tab. The compose window is handled via `PopOutMessage` dispatch with handler methods extracted to `crates/app/src/handlers/contacts.rs`.

### c. Token-to-Catalog theming (named style classes)

**Not applicable -- token input uses raw renderer drawing.** The token input widget draws directly via `renderer.fill_quad()` and `renderer.fill_text()` with palette colors, bypassing iced's styling system entirely. This is architecturally correct for a custom `advanced::Widget`.

### d. iced_drop drag-and-drop

**Not implemented.** `iced_drop` is not a dependency. No drag-and-drop for tokens between To/Cc/Bcc fields. No DnD in the group editor.

### e. Subscription orchestration

**No subscriptions used for contacts.** Contacts are loaded on-demand, not via streaming/polling. The GAL cache polling (spec calls for hourly refresh) would need subscriptions but is not implemented.

### f. Core CRUD bypassed

**Yes -- the app crate has its own raw SQL for contact operations.** `crates/app/src/db/contacts.rs` contains autocomplete search, contact/group CRUD, all writing raw SQL directly. Meanwhile, `crates/core/src/db/queries_extra/contacts.rs` and `contact_groups.rs` have parallel sets. This violates the core crate boundary principle.

### g. Dead code

Dead code has been cleaned up:
- `ContactSearchResult` and `ContactSearchKind` removed from `token_input.rs` (were never used).
- `RecipientField` is now used by `AutocompleteState` and `TokenContextMenuState`.
- `search_contacts_for_autocomplete` is now called from `handlers/contacts.rs` via `Db::search_autocomplete`.
- `AUTOCOMPLETE_MAX_HEIGHT` and `AUTOCOMPLETE_ROW_HEIGHT` are now used by the autocomplete dropdown view.
