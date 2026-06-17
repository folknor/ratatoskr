# Signatures: Spec vs. Code Discrepancies

Audit date: 2026-05-15 (updated 2026-05-18)

---

## Resolved (previously open)

- Signature editor now uses rich text editor (EditorState) instead of undoable_text_input
- Formatting toolbar implemented (B/I/U/S, lists, blockquote)
- Drag reorder grip handles implemented on signature rows
- Account-switch signature replacement wired (resolve via db_resolve_signature_for_compose, apply via replace_signature, ComposeState tracks active_signature_id + signature_separator_index)
- App handlers now use core CRUD (db_insert_signature, db_update_signature, db_delete_signature) instead of raw SQL
- Signature edit detection flag (`dirty: bool` on `SignatureEditorState`) [x]
- Signature list redesigned to mirror the Accounts list: flat list, color-dot + signature name, single "+ Add Signature" button at the bottom. Drag-reorder UI removed (dead `SignatureDragState` plumbing deleted).
- Signature editor rebuilt on the standard `setting_row` primitives - Account picker (new icon-capable `widgets::select_with_icons`), Name input via `input_row`, locked account for existing signatures (disabled-dropdown variant: no chevron, dimmed label). RTE body adopts the recessed `ContainerClass::EmailBody` + `PAD_CONTENT` styling from the compose pop-out.
- **Draft restoration with signature state** - both `signature_id` and `signature_separator_index` are persisted columns on `local_drafts` (`crates/db/src/db/schema/04_compose.sql:62-68`); `SaveLocalDraftParams` (`crates/db/src/db/queries_extra/compose.rs:545`) carries them; `compose_draft.rs:53,56` populates them from `ComposeState`; and `ComposeState::from_draft` (`crates/app/src/pop_out/compose/state.rs:196,238`) reads them back on reopen. Verified 2026-05-18.

## Remaining

(none)

## Superseded

### Per-account default signature dropdown in Account Settings
Originally proposed as a second surface for assigning per-account defaults. Superseded by the in-editor toggles ("New messages" / "Replies & forwards"): saving a signature with either flag set runs a DB transaction that first clears the same flag on every other signature for the same account (`db_insert_signature_sync` / `db_update_signature_sync` in `crates/db/src/db/queries_extra/compose.rs`), and the post-ack re-list refreshes the UI. A duplicate dropdown in Account Settings would be a redundant entry point onto the same state.

## Not a discrepancy

### SignatureEditorMessage flattened into SettingsMessage
Spec proposed a dedicated enum. Code puts editor messages directly in SettingsMessage. Functionally equivalent.
