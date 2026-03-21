# Signatures: Spec vs Implementation Discrepancies

Audit date: 2026-03-21
Last updated: 2026-03-21

Spec: `docs/signatures/implementation-spec.md`

---

## What matches the spec

### Phase 1 — Data model (complete)

- **`DbSignature` struct extended with all sync columns.** The struct in
  `crates/db/src/db/types.rs` now includes `body_text`, `is_reply_default`,
  `source`, `server_id`, `server_html_hash`, `last_synced_at`, `created_at`.
  The `FromRow` impl in `from_row_impls.rs` reads all columns.

- **All CRUD functions exist and match spec.** The core CRUD in
  `crates/core/src/db/queries_extra/compose.rs` includes:
  `db_get_signatures_for_account`, `db_get_all_signatures`,
  `db_get_default_signature`, `db_get_reply_signature`,
  `db_insert_signature`, `db_update_signature`, `db_delete_signature`,
  `db_reorder_signatures`, `db_set_reply_default_signature`,
  `db_resolve_signature_for_compose`.

- **Transactional default management** in insert and update matches the spec.
  Both `is_default` and `is_reply_default` are cleared for the same account
  in a transaction when setting a new default.

- **`html_to_plain_text` implemented** in
  `crates/core/src/db/queries_extra/compose.rs`. Uses `lol_html` to insert
  newlines for block elements, then strips tags for plain-text output.

- **`body_text` auto-generated on save.** The handler in
  `crates/app/src/handlers/signatures.rs` calls `html_to_plain_text()` when
  saving a signature and stores the result in `body_text`.

### Phase 2 — Settings UI (complete)

- **Signature list section** is implemented in `crates/app/src/ui/settings/tabs.rs`
  (`signature_list_section`). Signatures are grouped by account with account
  header rows, per-account "Add Signature" buttons, and per-signature
  edit/delete actions.

- **Signature editor overlay** exists (`signature_editor_overlay` in
  `tabs.rs`). It has: name field, default checkboxes (both "new messages" and
  "replies & forwards"), body field, save/delete buttons with delete
  confirmation.

- **Delete confirmation implemented.** Clicking delete (from list row or
  editor) opens the editor overlay with a "Delete this signature? Cancel /
  Confirm" prompt instead of deleting immediately.

- **SignatureEditorState** exists in `crates/app/src/ui/settings/types.rs`.

- **SignatureSaveRequest** and **SettingsEvent::SaveSignature /
  DeleteSignature** exist, enabling upward event emission to the App.

- **Settings component** implements `Component` trait correctly.

- **Signature loading is async.** Uses `Task::perform` via
  `handlers::signatures::load_signatures_async()`. No more synchronous
  loading on the UI thread.

### Phase 3 — Compose document assembly (complete)

- **`assemble_compose_document`** is fully implemented in
  `crates/rich-text-editor/src/compose.rs`.

- **`ComposeDocumentAssembly`** struct now includes `active_signature_id`
  alongside `document` and `signature_separator_index`.

- **Signature manipulation helpers** (`insert_signature`,
  `remove_signature`, `replace_signature`) are implemented with thorough
  tests (15+ test cases).

### Phase 5 — Send path (partial)

- **`finalize_compose_html`** implemented in
  `crates/core/src/db/queries_extra/compose.rs`. Wraps the signature region
  in `<div id="ratatoskr-signature">`.

- **`finalize_compose_plain_text`** implemented. Inserts RFC 3676 `-- \n`
  separator before signature text.

### Provider sync infrastructure

- Gmail signature sync (`crates/gmail/src/sync/labels.rs`) exists.
- JMAP signature sync (`crates/jmap/src/signatures.rs`) exists.
- Inline image extraction (`crates/provider-utils/src/signature_images.rs`)
  exists.

### Cross-cutting — Core CRUD used (not bypassed)

- **App handlers use proper transactional CRUD.** The raw SQL in `main.rs`
  has been replaced with `crates/app/src/handlers/signatures.rs` which uses
  transactional default-clearing for both `is_default` and
  `is_reply_default`. The handler is wired via `Message::SignatureOp`.

---

## What diverges from the spec

### Phase 2 — UI divergences

1. **Signature editor uses plain `text_input`, not the rich text editor.**
   The spec requires the signature editor to use the `RichTextEditor` widget
   (with formatting toolbar, HTML round-trip via `Document`). The actual
   implementation uses a basic `undoable_text_input` for the body field.
   Users must type raw HTML. This is a V1 limitation pending editor Phase 3.

2. **No formatting toolbar.** The spec calls for a formatting toolbar
   (B/I/U/S, lists, blockquote, link). This is absent pending the rich text
   editor integration.

3. **No drag reorder grip handles.** The spec shows grip handles on
   signature rows for reordering. The `db_reorder_signatures` query exists
   but is not wired to UI drag handlers.

4. **`SignatureEditorMessage` is flattened into `SettingsMessage`.** The spec
   proposes a dedicated `SignatureEditorMessage` enum. The actual code
   flattens all editor messages directly into `SettingsMessage`. This is a
   minor structural divergence — functionally equivalent.

### Phase 4 — Account switching (not implemented)

5. **No account-switch signature replacement.** The
   `handle_from_account_changed` flow, `replace_signature` integration in
   compose state, and confirmation dialog for edited signatures are not
   implemented in the app crate. The compose window itself is a stub.

### Phase 5 — Send path (partial)

6. **No draft restoration with signature state.** Draft persistence does not
   reconstruct `signature_separator_index` from saved HTML.

---

## What is missing entirely

| Spec item | Status |
|-----------|--------|
| Phase 2.2: Rich text editor in signature editor | Not done (plain text input; blocked on editor Phase 3) |
| Phase 2.2: Formatting toolbar | Not done (blocked on editor Phase 3) |
| Phase 2.3: Per-account default signature dropdown in Account Settings | Not done (blocked on account settings impl) |
| Phase 3.1: `ComposeDocumentState` in app crate | Not done (compose window is a stub) |
| Phase 3.1: Signature edit detection (`signature_edited` flag) | Not done |
| Phase 4: Account-switching signature replacement + confirmation dialog | Not done (compose window is a stub) |
| Phase 5.3: Draft restoration with signature state | Not done |

---

## Cross-cutting concerns

### a. Generational load tracking

**Not used.** Signature loading uses async `Task::perform` via
`handlers::signatures::load_signatures_async()`. No generation counters.

### b. Component trait

**Implemented.** `Settings` implements `Component` with signature
save/delete emitted upward to the App via `SettingsEvent`.

### c. Token-to-Catalog theming (named style classes)

**Used throughout.** No raw color values in signature UI code.

### d. iced_drop drag-and-drop (signature reorder)

**Not implemented in UI.** The `db_reorder_signatures` query exists in core
but grip handles and drag support are not wired for signature rows.

### e. Core CRUD properly used

**Yes.** The raw SQL bypass has been eliminated. All signature operations
go through `crates/app/src/handlers/signatures.rs` which uses proper
transactional semantics for default-clearing.

### f. Dead code

**Reduced.** The core CRUD functions are no longer dead code — they are
used by provider sync and the `db_resolve_signature_for_compose` function.
The `html_to_plain_text` function is called from the app handler.
