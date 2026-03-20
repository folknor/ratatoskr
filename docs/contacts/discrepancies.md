# Contacts Feature: Spec vs Implementation Discrepancies

Audit date: 2026-03-21. Covers `problem-statement.md`, `import-spec.md`, and `autocomplete-implementation-spec.md`.

---

## What matches the spec

### Token input widget (`crates/app/src/ui/token_input.rs`)

- **Custom `advanced::Widget`** implementation as specified. File location matches spec (`crates/app/src/ui/token_input.rs`).
- **Data types match**: `Token`, `TokenId`, `RecipientField`, `TokenInputValue` all present with the specified fields (`id`, `email`, `label`, `is_group`, `group_id`). `TokenInputValue` has `tokens`, `text`, `next_id` as specified.
- **Wrapping flow layout** implemented correctly: tokens laid out left-to-right with row wrapping, text input placed after tokens, wraps to new row if remaining space < `TOKEN_TEXT_MIN_WIDTH`.
- **Layout constants** all present in `crates/app/src/ui/layout.rs`: `TOKEN_HEIGHT` (24), `TOKEN_RADIUS`, `PAD_TOKEN`, `TOKEN_SPACING`, `TOKEN_ROW_SPACING`, `PAD_TOKEN_INPUT`, `TOKEN_TEXT_MIN_WIDTH` (120), `TOKEN_GROUP_ICON_SIZE`, `AUTOCOMPLETE_MAX_HEIGHT` (300), `AUTOCOMPLETE_ROW_HEIGHT` (32).
- **Keyboard state machine** implemented: backspace-at-start selects last token, backspace-with-selected removes it, comma/semicolon always tokenize, space tokenizes if text contains `@`, Enter/Tab tokenize, Escape blurs.
- **Paste handling** implemented: Ctrl+V/Cmd+V detected, clipboard read, `Paste` message emitted.
- **Token chip drawing**: background quad with rounded corners, selected/hovered states with palette colors, label text rendering.
- **Focus management**: click on token selects + focuses, click in field focuses, click outside blurs.
- **Email validation**: `is_plausible_email()` helper present with basic `@` + `.` check.

### Contact search types

- `ContactSearchResult`, `ContactSearchKind` (Person/Group with `group_id` + `member_count`) defined in `token_input.rs`.
- `ContactMatch` type in `crates/app/src/db/contacts.rs` for autocomplete search.

### Compose integration (`crates/app/src/pop_out/compose.rs`)

- To/Cc/Bcc fields each use `TokenInputValue` with per-field selected token state.
- `ComposeMessage` has `ToTokenInput`, `CcTokenInput`, `BccTokenInput` variants.
- `handle_token_input_message` dispatches all `TokenInputMessage` variants.
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
- Delete for both contacts and groups.

---

## Divergences from spec

### Token input widget

1. **Missing `TokenInputMessage` variants**: The spec defines `TokenContextMenu(TokenId, Point)`, `DragStarted(TokenId)`, `TokenDropped { token, source_field }`, and `MoveToken { token_id, target_field }`. The implementation has none of these. Instead it has `BackspaceAtStart` (not in spec).

2. **No `DragState`**: The spec defines `DragState { token_id, origin, current }` in `TokenInputState`. The implementation has no drag state tracking at all.

3. **Simplified `TokenInputState`**: Spec has `selection: TokenSelection`, `cursor_position: usize`, `drag: Option<DragState>`. Implementation only has `token_bounds: Vec<Rectangle>` and `is_focused: bool`. Token selection is managed externally via `selected_token` parameter rather than internally.

4. **Widget constructor signature differs**: Spec defines `token_input(field, tokens, text, placeholder, on_message)` with `RecipientField` parameter. Implementation is `token_input_field(tokens, text, placeholder, selected_token, on_message)` -- no `field` parameter, adds `selected_token` parameter instead.

5. **Text measurement**: Spec calls for `renderer.measure()` for precise token widths. Implementation uses a character-width heuristic (`label.len() * TEXT_MD * 0.54`), noted as "adequate for layout."

6. **No right-click handling**: The widget does not handle `mouse::Button::Right` events at all. No context menu support.

7. **Arrow key navigation not implemented**: Spec describes left/right arrow navigation between tokens. Not present in the implementation.

8. **Group icon on group tokens**: Spec says group tokens should have a people icon prepended + member count suffix (e.g., "Engineering (12)"). The draw code does not distinguish group tokens from person tokens visually.

### Autocomplete dropdown

9. **Not implemented at all**: The spec's Phase 2 (autocomplete dropdown) is entirely missing from the compose view. The compose `update()` function does not call `search_contacts_for_autocomplete` or any search function. There is no `AutocompleteState` in `ComposeState`. No dropdown rendering, no keyboard navigation interception for dropdown, no debounced search.

10. **`search_contacts_unified()` not implemented**: The spec defines this function in `crates/core/src/contacts/search.rs` with `ContactSearchMode` (All/GroupsOnly). No such module exists. The app-layer `search_contacts_for_autocomplete` in `crates/app/src/db/contacts.rs` is a simpler version (LIKE-only, no FTS, no groups, no GAL) and is never called from compose.

11. **`ContactSearchResult` and `ContactSearchKind` are dead code**: Defined in `token_input.rs` but never imported or used anywhere in the codebase.

### Paste handling

12. **No RFC 5322 parsing**: Spec calls for a dedicated `parse_pasted_addresses()` function in `crates/app/src/ui/token_input_parse.rs` that handles `Name <email>`, `"Name" <email>`, bare email formats with display name extraction. Implementation just splits on `,;\\n` and uses the raw text as both `email` and `label`. No display name extraction.

13. **No paste deduplication**: Spec requires dedup within paste and against existing tokens. Implementation does neither.

14. **No email validation on paste**: Spec says invalid addresses should be silently dropped. Implementation tokenizes everything, including non-email text.

15. **No bulk paste banner**: Spec calls for a Bcc/group-creation suggestion banner when 10+ addresses are pasted. Not implemented.

### Contact management UI

16. **No account selector on contact creation**: Spec says first field in the contact creation editor is an account selector dropdown. Not implemented -- new contacts have no account assignment UI.

17. **No confirmation on delete**: Spec requires deletion to prompt for confirmation. Implementation immediately triggers delete.

18. **No distinction between local and synced save behavior**: Spec says local contacts save immediately on edit, synced contacts use explicit Save button. Implementation always uses an explicit Save button regardless of source.

19. **No provider write-back on save**: Spec says synced contact edits are pushed to the provider API. Implementation saves locally only.

20. **Inline contact editing popover**: Spec describes a popover on reading pane sender/recipient pills for quick contact editing. Not implemented.

### Contact import

21. **Entire import feature missing**: The spec defines a `crates/contact-import/` crate for CSV/XLSX/vCard import with encoding detection, column mapping, preview, and an import wizard UI. No such crate exists. No import UI in settings.

### GAL caching

22. **GAL/directory caching not implemented**: Spec describes pre-fetching organization directory at startup with polling refresh. No GAL cache implementation found in the codebase.

---

## Cross-cutting concerns

### a. Generational load tracking

**Not used in contacts scope.** The autocomplete spec defines `search_generation: u64` in `AutocompleteState` for discarding stale search results, mirroring the pattern used elsewhere in the app (`nav_generation`, `thread_generation`, `search_generation` in `main.rs`). Since autocomplete search is not wired up at all, no generation tracking exists for contacts.

### b. Component trait

**Contacts UI is not componentized.** The settings panel (`Settings`) implements `Component`, and contacts management lives within it as part of the People tab. However, contacts do not have their own `Component` impl -- they are inline within the settings update/view. The compose window is also not a `Component` -- it is handled via `PopOutMessage` dispatch. The spec envisions the compose view as a `Component` (`ComposeView (Component)` in the architecture diagram), but the implementation uses a flat update function (`update_compose`).

### c. Token-to-Catalog theming (named style classes)

**Not applicable -- token input uses raw renderer drawing.** The token input widget draws directly via `renderer.fill_quad()` and `renderer.fill_text()` with palette colors, bypassing iced's styling system entirely. This is architecturally correct for a custom `advanced::Widget` (there are no iced widgets to apply style classes to). The compose view uses named style classes (`theme::ContainerClass`, `theme::ButtonClass`, `theme::TextClass`, `theme::PickListClass`) appropriately.

### d. iced_drop drag-and-drop

**Not implemented.** `iced_drop` is not a dependency. No drag-and-drop for tokens between To/Cc/Bcc fields. No DnD in the group editor. The spec explicitly calls for both.

### e. Subscription orchestration

**No subscriptions used for contacts.** The contacts feature does not use iced subscriptions. This is expected -- contacts are loaded on-demand via `SettingsEvent::LoadContacts`, not via streaming/polling. The GAL cache polling (spec calls for hourly refresh) would need subscriptions but is not implemented.

### f. Core CRUD bypassed

**Yes -- the app crate has its own raw SQL for contact operations.** `crates/app/src/db/contacts.rs` contains `search_contacts_for_autocomplete`, `load_contacts_filtered`, `load_groups_filtered`, `save_contact_inner`, `save_group_inner`, and delete operations, all writing raw SQL directly against the database via the app's `Db` wrapper. Meanwhile, `crates/core/src/db/queries_extra/contacts.rs` has its own parallel set of CRUD functions (`db_upsert_contact`, `db_update_contact`, `db_delete_contact`, etc.) and `crates/core/src/db/queries_extra/contact_groups.rs` has another parallel set. This is a clear violation of the core crate boundary principle: business logic belongs in `ratatoskr-core`, not duplicated in the app crate with raw SQL.

### g. Dead code

Several items are defined but never used:

1. **`ContactSearchResult`** and **`ContactSearchKind`** in `token_input.rs` -- never imported or referenced outside their definition file.
2. **`RecipientField`** enum in `token_input.rs` -- defined but never used (the widget constructor does not take a `field` parameter, and compose does not reference it).
3. **`search_contacts_for_autocomplete`** in `crates/app/src/db/contacts.rs` -- exported from `db/mod.rs` but never called from compose or anywhere else in the app.
4. **`AUTOCOMPLETE_MAX_HEIGHT`** and **`AUTOCOMPLETE_ROW_HEIGHT`** in `layout.rs` -- defined but no autocomplete dropdown exists to use them.
