# Signatures: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Phase 2 — UI

**Signature editor uses plain `undoable_text_input`, not the rich text editor.** The editor overlay at `crates/app/src/ui/settings/tabs.rs:1067-1087` uses `undoable_text_input` for the body field. Users must type raw HTML. The `RichTextEditor` widget exists in `crates/rich-text-editor/` but is not wired to the signature editor. Status: **not implemented**.

**No formatting toolbar in signature editor.** Spec calls for B/I/U/S, lists, blockquote, link. Absent. Status: **not implemented** (blocked on rich text editor integration).

**No drag reorder grip handles.** `db_reorder_signatures` exists at `crates/core/src/db/queries_extra/compose.rs:273` but no UI drag handles for signature rows. Status: **not implemented** (query exists, UI absent).

**`SignatureEditorMessage` flattened into `SettingsMessage`.** Spec proposes dedicated enum. Actual code puts all editor messages directly in `SettingsMessage` at `crates/app/src/ui/settings/types.rs:98-107`. Functionally equivalent. Status: **minor structural divergence**.

### Phase 4 — Account switching

**No account-switch signature replacement.** No `handle_from_account_changed` signature flow, no `replace_signature` integration in compose, no confirmation dialog. Status: **not implemented**.

### Phase 5 — Send path (partial)

**No draft restoration with signature state.** Draft persistence does not reconstruct `signature_separator_index` from saved HTML. Status: **not implemented**.

### Core CRUD bypassed

**App handlers use raw SQL, not core CRUD functions.** `crates/app/src/handlers/signatures.rs` implements save/delete/load via raw SQL in `Db::with_write_conn`. It does NOT call `db_insert_signature()`, `db_update_signature()`, or `db_delete_signature()` from `crates/core/src/db/queries_extra/compose.rs`. It does call `html_to_plain_text()` from core (`signatures.rs:214`). Core CRUD functions are used only by provider sync. Status: **architectural divergence**.

---

## Implemented and wired

### Phase 1 — Data model

- **`DbSignature` struct** in `crates/db/src/db/types.rs:536` includes all v41 columns: `body_text`, `is_reply_default`, `source`, `server_id`, `server_html_hash`, `last_synced_at`, `created_at`.
- **Core CRUD functions** at `crates/core/src/db/queries_extra/compose.rs:91-327`: `db_get_signatures_for_account`, `db_get_all_signatures`, `db_get_default_signature`, `db_get_reply_signature`, `db_insert_signature`, `db_update_signature`, `db_delete_signature`, `db_reorder_signatures`, `db_set_reply_default_signature`, `db_resolve_signature_for_compose`.
- **Transactional default management** in core insert/update: clears `is_default` and `is_reply_default` for same account in transaction.
- **`html_to_plain_text`** at `compose.rs:716` using `lol_html`.

### Phase 2 — Settings UI

- **Signature list section** at `tabs.rs:871`: grouped by account, per-account "Add Signature" buttons, per-signature edit/delete actions.
- **Signature editor overlay** at `tabs.rs:1016`: name field, default checkboxes (new messages + replies/forwards), body field (plain text), save/delete with delete confirmation.
- **`SignatureEditorState`** at `types.rs:320`.
- **`SignatureSaveRequest`** and **`SettingsEvent::SaveSignature/DeleteSignature`** at `types.rs:308,160-162`.
- **Settings `Component` trait** impl at `update.rs:14`.
- **Async signature loading** via `handlers::signatures::load_signatures_async()` at `signatures.rs:119`.
- **App handler dispatches** signature save/delete/load at `handlers/signatures.rs:20-157`, wired via `Message::SignatureOp` at `main.rs:237` and `SettingsEvent` at `main.rs:1128-1132`.

### Phase 3 — Compose document assembly

- **`assemble_compose_document`** at `crates/rich-text-editor/src/compose.rs:52`.
- **`ComposeDocumentAssembly`** struct at `compose.rs:23` with `document`, `signature_separator_index`, `active_signature_id`.
- **Signature manipulation helpers** (`insert_signature`, `remove_signature`, `replace_signature`) at `compose.rs:157,186,207`.

### Phase 5 — Send path (partial)

- **`finalize_compose_html`** at `crates/core/src/db/queries_extra/compose.rs:801`: wraps signature in `<div id="ratatoskr-signature">`.
- **`finalize_compose_plain_text`** at `compose.rs:842`: inserts RFC 3676 `-- \n` separator.

### Provider sync

- Gmail signature sync at `crates/gmail/src/sync/labels.rs:155` (`sync_signatures`).
- JMAP signature sync at `crates/jmap/src/signatures.rs`.
- Inline image extraction at `crates/provider-utils/src/signature_images.rs`.

---

## Not implemented

| Spec item | Status |
|-----------|--------|
| Phase 2.2: Rich text editor in signature editor | Not implemented (plain text input; blocked on editor integration) |
| Phase 2.2: Formatting toolbar | Not implemented (blocked on editor integration) |
| Phase 2.3: Per-account default signature dropdown in Account Settings | Not implemented |
| Phase 3.1: `ComposeDocumentState` in app crate | Not implemented (compose uses `text_editor::Content`, not `Document`) |
| Phase 3.1: Signature edit detection (`signature_edited` flag) | Not implemented |
| Phase 4: Account-switching signature replacement + confirmation dialog | Not implemented |
| Phase 5.3: Draft restoration with signature state | Not implemented |
