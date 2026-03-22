# Contacts: Spec vs. Code Discrepancies

Audit date: 2026-03-22

---

## Divergences

### Autocomplete — dropdown wired but provider clients not yet connected for GAL

Autocomplete search is now wired end-to-end: `dispatch_autocomplete_search()` is called from `pop_out.rs` when `should_trigger_autocomplete()` returns true. Dropdown renders, keyboard navigation (ArrowUp/Down/Enter/Tab/Escape) intercepts correctly via `autocomplete_open` flag on token input widget. GAL cache table exists (`gal_cache`) and is searched during autocomplete, but GAL pre-fetch requires provider client access (Graph `/users`, Google Directory API) which awaits sync orchestrator integration.

### Provider write-back — scaffolding only

Core has `ContactUpdate`, `ContactSource`, `WriteBackResult` types in `contacts/save.rs`. Google and Graph body builders + server info lookups exist. JMAP `ContactCard/set` is fully implemented. CardDAV PUT not yet added. App dispatches write-back after contact save (`dispatch_provider_write_back` in `handlers/contacts.rs`) but actual HTTP calls are logged as info-level stubs until provider clients are available.

### XLSX import — deferred

Spec requires XLSX support. Only CSV and vCard are implemented in `crates/contact-import/`. OOXML infrastructure exists in the squeeze crate but is not integrated. Explicitly deferred per user decision.

---

## Implemented and wired

### Token input widget
Custom `advanced::Widget` with wrapping flow layout, keyboard state machine (backspace/comma/Tab/Enter/Escape), arrow key navigation, right-click context menu, group icon rendering, paste message, email validation, drag detection (4px threshold).
- Code: `crates/app/src/ui/token_input.rs`

### Autocomplete dropdown
Triggered on text change in To/Cc/Bcc fields. `dispatch_autocomplete_search()` called from `pop_out.rs` compose handler. Generation-tracked results. Keyboard navigation (Up/Down/Enter/Tab/Escape) via `autocomplete_open` flag. Active field tracks which field to push tokens to.
- Code: `crates/app/src/handlers/contacts.rs:223`, `crates/app/src/pop_out/compose.rs`

### RFC 5322 paste parser
`parse_pasted_addresses()` properly wired in `handle_token_input_message` Paste handler. Handles bare email, name+angle-bracket, quoted name, comma/semicolon/newline separated, mixed formats. 7 unit tests.
- Code: `crates/app/src/ui/token_input_parse.rs`, `crates/app/src/pop_out/compose.rs:1088`

### Token context menu
Right-click on token opens context menu with: Delete, Expand group (group tokens only), Move to To/Cc/Bcc (other fields). Group expansion via `expand_contact_group()` with recursive nested group handling and display name lookup.
- Code: `crates/app/src/pop_out/compose.rs` (TokenContextMenuState, context menu rendering)

### Token drag-and-drop
Drag detection in token input widget (4px threshold). `DragStarted(TokenId)` message emitted. Compose state tracks `ComposeTokenDrag` with source field and label. Context menu "Move to" provides the primary cross-field move mechanism.
- Code: `crates/app/src/ui/token_input.rs`, `crates/app/src/pop_out/compose.rs`

### Bcc nudge banner
When a group token is added to To or Cc via autocomplete, a dismissible banner suggests moving to Bcc. Accept moves the token; dismiss removes the banner.
- Code: `crates/app/src/pop_out/compose.rs` (BccNudgeBanner, bcc_nudge_banner view)

### Bulk paste banner
When paste tokenizes 10+ addresses, a dismissible banner appears suggesting saving as a contact group.
- Code: `crates/app/src/pop_out/compose.rs` (BulkPasteBanner, bulk_paste_banner_view)

### Group creation from import
`execute_contact_import()` now creates `contact_groups` entries and links `contact_group_members` for groups found in imported contacts. Groups split by `;`, `,`, `|` delimiters in the import data.
- Code: `crates/app/src/handlers/contacts.rs:189-251`

### Contact management UI
Contact/group lists with filter inputs, slide-in editor overlays, full CRUD with delete confirmation. Styled group pills (Badge container class) and account source pills on contact cards.
- Code: `crates/app/src/ui/settings/tabs.rs`

### Distinct save behavior (local vs synced)
Local contacts (`source = None or "user"`) auto-save on field change when editing existing contacts. Synced contacts show explicit Save button, enabled only when dirty. New contacts always show Create button.
- Code: `crates/app/src/ui/settings/update.rs`, `crates/app/src/ui/settings/tabs.rs`

### Inline contact editing from reading pane
Sender name in expanded message card is clickable. Clicking opens Settings > People with the contact editor for that email address (existing contact) or a new editor pre-populated with the email.
- Code: `crates/app/src/ui/widgets.rs:1028`, `crates/app/src/main.rs:open_contact_editor_for_email`

### GAL cache infrastructure
`gal_cache` table (migration v66) with email, display_name, phone, company, title, department, account_id. Core functions: `cache_gal_entries()`, `gal_cache_age()`. App autocomplete includes GAL search via `search_gal_cache()`. Hourly polling subscription and boot-time trigger for cache refresh.
- Code: `crates/core/src/contacts/gal.rs`, `crates/app/src/db/contacts.rs`, `crates/db/src/db/migrations.rs`

### Backend data layer
- `crates/seen-addresses/`: direction scoring
- `crates/core/src/db/queries.rs`: `search_contacts()` with FTS5 + LIKE fallback
- `crates/core/src/db/queries_extra/contact_groups.rs`: full CRUD, recursive expansion with cycle detection
- `crates/core/src/db/queries_extra/contacts.rs`: contact CRUD, stats, avatar updates
- `crates/core/src/contacts/save.rs`: dual save pattern (local immediate, synced held)
- `crates/core/src/contacts/gal.rs`: GAL cache persistence
- `crates/app/src/db/contacts.rs`: app-level autocomplete search, contact/group CRUD, GAL search
- `crates/core/src/carddav/`: CardDAV sync
- `crates/core/src/contact_photos.rs`: contact photos
