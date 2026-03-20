# Signatures: Spec vs Implementation Discrepancies

Audit date: 2026-03-21

Spec: `docs/signatures/implementation-spec.md`

---

## What matches the spec

### Phase 1 — Data model (partial)

- **Basic CRUD exists and matches spec.** All five functions listed in the spec
  are present in `crates/core/src/db/queries_extra/compose.rs`:
  `db_get_signatures_for_account`, `db_get_default_signature`,
  `db_insert_signature`, `db_update_signature`, `db_delete_signature`.
- **Transactional default management** in insert and update matches the spec
  (clears old default in a transaction when `is_default = true`).

### Phase 2 — Settings UI (partial)

- **Signature list section** is implemented in `crates/app/src/ui/settings/tabs.rs`
  (`signature_list_section`). Signatures are grouped by account with account
  header rows, per-account "Add Signature" buttons, and per-signature
  edit/delete actions. This matches the spec's layout.
- **Signature editor overlay** exists (`signature_editor_overlay` in
  `tabs.rs`). It has: name field, default checkboxes (both "new messages" and
  "replies & forwards"), body field, save/delete buttons. The overlay enum
  `SettingsOverlay::EditSignature { signature_id, account_id }` matches the
  spec. `SettingsOverlay` is `Clone` (not `Copy`), as spec required.
- **SignatureEditorState** exists in `crates/app/src/ui/settings/types.rs` with
  `signature_id`, `account_id`, `name`, `body`, `is_default`,
  `is_reply_default`.
- **SignatureSaveRequest** and **SettingsEvent::SaveSignature /
  DeleteSignature** exist, enabling upward event emission to the App.
- **Settings component** implements `Component` trait (`impl Component for
  Settings`), matching the project's component pattern.

### Phase 3 — Compose document assembly (implemented)

- **`assemble_compose_document`** is fully implemented in
  `crates/rich-text-editor/src/compose.rs`. The function signature differs
  slightly from spec (takes `Option<&str>` for signature HTML instead of
  `Option<&DbSignature>`), but the behavior matches: empty paragraph, optional
  HR separator + signature blocks, optional attribution + blockquote.
- **`ComposeDocumentAssembly`** struct matches spec
  (`document` + `signature_separator_index`), though it omits
  `active_signature_id` (the spec included it).
- **Signature manipulation helpers** (`insert_signature`,
  `remove_signature`, `replace_signature`) are implemented with thorough tests
  (15+ test cases covering edge cases, blank handling, index clamping).
- **`QuotedContent`** struct matches spec.
- **Helper builders** (`build_reply_attribution_block`,
  `build_forward_header`) match spec.

### Provider sync infrastructure

- Gmail signature sync (`crates/gmail/src/sync/labels.rs`) exists.
- JMAP signature sync (`crates/jmap/src/signatures.rs`) exists.
- Inline image extraction (`crates/provider-utils/src/signature_images.rs`)
  exists.

---

## What diverges from the spec

### Phase 1 — Data model divergences

1. **`DbSignature` struct is incomplete.** The spec (Phase 1.1) calls for
   extending `DbSignature` with sync columns: `body_text`, `is_reply_default`,
   `source`, `server_id`, `server_html_hash`, `last_synced_at`, `created_at`.
   The actual struct in `crates/db/src/db/types.rs` still has only the original
   6 fields (`id`, `account_id`, `name`, `body_html`, `is_default`,
   `sort_order`). The `FromRow` impl in `from_row_impls.rs` also only reads
   these 6 columns.

2. **SELECT queries are incomplete.** The CRUD queries in `compose.rs` only
   select the 6 original columns. The v41 columns (`is_reply_default`,
   `body_text`, `source`, etc.) are never read from the database through the
   typed query layer.

3. **`is_reply_default` handling is bypassed.** The core CRUD functions
   `db_insert_signature` and `db_update_signature` do not handle
   `is_reply_default`. However, the app's raw SQL in `main.rs` DOES write
   `is_reply_default` (see "Core CRUD bypassed" below).

### Phase 2 — UI divergences

4. **Signature editor uses plain `text_input`, not the rich text editor.**
   The spec requires the signature editor to use the `RichTextEditor` widget
   (with formatting toolbar, HTML round-trip via `Document`). The actual
   implementation uses a basic `undoable_text_input` for the body field, with
   the label "Signature body (HTML)". Users must type raw HTML. This is
   explicitly noted in the code as "plain text for V1".

5. **No formatting toolbar.** The spec calls for a formatting toolbar
   (B/I/U/S, lists, blockquote, link) identical to the compose toolbar. This
   is absent.

6. **No drag reorder grip handles.** The spec shows grip handles (drag handle icon) on
   signature rows for reordering. The actual signature list rows have no grip
   handles and no drag reorder support.

7. **`SignatureEditorMessage` is flattened into `SettingsMessage`.** The spec
   proposes a dedicated `SignatureEditorMessage` enum wrapped under
   `SettingsMessage::SignatureEditorMsg(...)`. The actual code flattens all
   signature editor messages directly into `SettingsMessage` (e.g.,
   `SignatureEditorNameChanged`, `SignatureEditorBodyChanged`, etc.). This is a
   minor structural divergence — functionally equivalent.

8. **No `signatures_loaded` / `SignaturesLoaded` message.** The spec calls
   for async loading via `Task::perform` returning
   `SignaturesLoaded(Result<...>)`. The actual code loads signatures
   synchronously via `load_signatures_into_settings()` using
   `db.with_conn_sync()`. There is no async message variant.

### Phase 3 — Compose integration divergences

9. **`ComposeDocumentAssembly` omits `active_signature_id`.** The spec
   includes it so the compose window can track which signature was inserted.
   The implementation returns only `document` + `signature_separator_index`.

### Phase 4 — Account switching (not implemented)

10. **No account-switch signature replacement.** The
    `handle_from_account_changed` flow, `replace_signature` integration in
    compose state, and confirmation dialog for edited signatures are not
    implemented in the app crate.

### Phase 5 — Send path (not implemented)

11. **No `finalize_compose_html`.** The spec's HTML post-processing to wrap
    signatures in `<div id="ratatoskr-signature">` does not exist.

12. **No `finalize_compose_plain_text`.** The RFC 3676 `-- \n` separator
    insertion in the plain-text alternative does not exist.

13. **No `html_to_plain_text`.** The spec's Phase 1.2 function for stripping
    HTML to plain text does not exist anywhere in the codebase (confirmed by
    search).

---

## What is missing entirely

| Spec item | Status |
|-----------|--------|
| Phase 1.1: Extended `DbSignature` with sync columns | Not done |
| Phase 1.2: `html_to_plain_text` function | Not done |
| Phase 1.3: `db_get_all_signatures` | Not done (raw SQL used instead) |
| Phase 1.3: `db_get_reply_signature` | Not done |
| Phase 1.3: `db_reorder_signatures` | Not done |
| Phase 1.3: `db_set_reply_default_signature` | Not done |
| Phase 1.4: `db_resolve_signature_for_compose` | Not done |
| Phase 2.2: Rich text editor in signature editor | Not done (plain text input used) |
| Phase 2.2: Formatting toolbar | Not done |
| Phase 2.3: Per-account default signature dropdown in Account Settings | Not done |
| Phase 3.1: `ComposeDocumentState` in app crate | Not done |
| Phase 3.1: Signature edit detection (`signature_edited` flag) | Not done |
| Phase 4: Account-switching signature replacement + confirmation dialog | Not done |
| Phase 5.1: `finalize_compose_html` (wrapper div) | Not done |
| Phase 5.2: `finalize_compose_plain_text` (RFC 3676 separator) | Not done |
| Phase 5.3: Draft restoration with signature state | Not done |

---

## Cross-cutting concerns

### a. Generational load tracking

**Not used.** Signature loading uses synchronous `with_conn_sync` in
`load_signatures_into_settings()`. No generation counters or load tracking
are involved. Data is reloaded after every save/delete via
`Message::ReloadSignatures`.

### b. Component trait

**Implemented.** `Settings` implements `Component` with `type Message =
SettingsMessage` and `type Event = SettingsEvent`. Signature save/delete
operations emit `SettingsEvent` variants upward to the App, which performs
DB operations. This follows the component pattern correctly.

### c. Token-to-Catalog theming (named style classes)

**Used.** The signature UI uses named style classes throughout:
`theme::ButtonClass::Action`, `theme::ButtonClass::Primary`,
`theme::ButtonClass::BareIcon`, `theme::TextInputClass::Settings`,
`theme::ContainerClass::Content`, `theme::TextClass::Tertiary`, and
`text::base` / `text::secondary` / `text::danger`. No raw color values.

### d. iced_drop drag-and-drop (signature reorder)

**Not implemented.** The spec calls for drag-reorder grip handles on
signature rows. While the Settings component has generic editable-list drag
infrastructure (`DragState`, `ListGripPress`, `ListDragMove`, etc.), this is
not wired up for signature rows. Signature rows have no grip handles and no
drag support. The `db_reorder_signatures` query also does not exist.

### e. Subscription orchestration

**Not used.** Signatures do not use iced subscriptions. Loading is
synchronous (triggered by account load and after mutations). This is
appropriate given the current design.

### f. Core CRUD bypassed

**Yes, bypassed.** The app's `main.rs` writes raw SQL for both signature
save and delete instead of calling the core CRUD functions in
`crates/core/src/db/queries_extra/compose.rs`:

- **Save** (around line 1853): raw `UPDATE signatures SET ...` / `INSERT
  INTO signatures ...` via `db.with_write_conn()`, including
  `is_reply_default` which the core CRUD doesn't support.
- **Delete** (around line 1899): raw `DELETE FROM signatures WHERE id = ?1`
  via `db.with_write_conn()`.
- **Load** (`load_signatures_into_settings`): raw `SELECT ... FROM
  signatures` via `with_conn_sync`, reading `is_reply_default` which is not
  in `DbSignature`.

The core CRUD functions (`db_insert_signature`, `db_update_signature`,
`db_delete_signature`) exist but are not called from the app. The raw SQL
in `main.rs` handles `is_reply_default` which the core functions do not.

### g. Dead code

**Yes, likely dead.** The core CRUD functions for signatures
(`db_insert_signature`, `db_update_signature`, `db_delete_signature`,
`db_get_signatures_for_account`, `db_get_default_signature`) are defined in
`crates/core/src/db/queries_extra/compose.rs` but are not called from the
app crate (confirmed by grep). They may be used by provider sync code, but
the app bypasses them entirely with raw SQL. The `DbSignature` type itself
may be unused in the app since `SignatureEntry` is used instead.
