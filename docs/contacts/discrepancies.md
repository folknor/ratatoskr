# Contacts: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Autocomplete — not wired

Autocomplete search never triggered: `dispatch_autocomplete_search()` and `should_trigger_autocomplete()` are defined but never called. No autocomplete dropdown rendering in `view_compose_window()`. `AutocompleteState` missing `active_field` -- `AutocompleteSelect` always pushes tokens to `state.to`. Keyboard interception for dropdown not implemented.
- Code: `crates/app/src/handlers/contacts.rs:103,133`, `crates/app/src/pop_out/compose.rs:106,359,475`

### Token input — paste parser not wired

RFC 5322 paste parser exists but is not called. `handle_token_input_message` handles `Paste` with simple `split([',', ';', '\n'])` instead. No bulk paste banner for 10+ addresses.
- Code: `crates/app/src/ui/token_input_parse.rs:26`, `crates/app/src/pop_out/compose.rs:450`

### Token input — drag and drop

No token drag-and-drop between fields. No `DragStarted`, `TokenDropped`, `MoveToken` messages. `iced_drop` is not a dependency.
- Code: `crates/app/src/ui/token_input.rs`

### Contact management UI

No distinction between local and synced save behavior -- `save_contact_inner` always uses UPSERT with no source check. No provider write-back on save. No inline contact editing popover on reading pane pills.
- Code: `crates/app/src/db/contacts.rs:445`, `crates/app/src/handlers/contacts.rs:38`

### Contact import

Entire import feature missing. Spec defines `crates/contact-import/` crate for CSV/XLSX/vCard import. No such crate exists.
- Spec: `docs/contacts/import-spec.md`

### GAL caching

GAL/directory caching not implemented. Spec describes pre-fetching organization directory at startup with polling refresh.
- Spec: `docs/contacts/autocomplete-implementation-spec.md`

### Crate boundary

App bypasses core CRUD for contacts. `crates/app/src/db/contacts.rs` has raw SQL for autocomplete search and contact/group CRUD. Core has parallel functions in `queries_extra/contacts.rs` and `contact_groups.rs`. `ContactMatch` type lives in app crate instead of core.
- Code: `crates/app/src/db/contacts.rs:9` (ContactMatch), `crates/core/src/db/queries_extra/contacts.rs`

## Dead code

- `RecipientField` defined but unused at `crates/app/src/ui/token_input.rs:50`
- `dispatch_autocomplete_search` and `should_trigger_autocomplete` never called at `crates/app/src/handlers/contacts.rs:103,133`
- `AUTOCOMPLETE_MAX_HEIGHT` and `AUTOCOMPLETE_ROW_HEIGHT` never used at `crates/app/src/ui/layout.rs:433,435`

---

## Implemented and wired

### Token input widget
Custom `advanced::Widget` with wrapping flow layout, keyboard state machine (backspace/comma/Tab/Enter/Escape), arrow key navigation, right-click context menu, group icon rendering, paste message, email validation.
- Code: `crates/app/src/ui/token_input.rs`

### Compose window
To/Cc/Bcc fields use `TokenInputValue`. Reply/ReplyAll/Forward populate tokens correctly. `AutocompleteState` struct exists with generation counter. Message variants for autocomplete results/select/navigate/dismiss exist and update state.
- Code: `crates/app/src/pop_out/compose.rs`

### Backend data layer
- `crates/seen-addresses/`: direction scoring
- `crates/core/src/db/queries.rs:680`: `search_contacts()` with FTS5 + LIKE fallback
- `crates/core/src/db/queries_extra/contact_groups.rs`: full CRUD, recursive expansion with cycle detection
- `crates/core/src/db/queries_extra/contacts.rs`: contact CRUD, stats, avatar updates
- `crates/app/src/db/contacts.rs`: app-level autocomplete search, contact/group CRUD
- `crates/core/src/carddav/`: CardDAV sync
- `crates/core/src/contact_photos.rs`: contact photos

### Contact management UI (Settings People tab)
Contact/group lists with filter inputs, slide-in editor overlays, full CRUD with delete confirmation.
- Code: `crates/app/src/ui/settings/tabs.rs:1146`, `crates/app/src/handlers/contacts.rs:13-97`
