# Signatures: Spec vs. Code Discrepancies

Audit date: 2026-03-23

---

## Resolved (previously open)

- Signature editor now uses rich text editor (EditorState) instead of undoable_text_input
- Formatting toolbar implemented (B/I/U/S, lists, blockquote)
- Drag reorder grip handles implemented on signature rows
- Account-switch signature replacement wired (resolve via db_resolve_signature_for_compose, apply via replace_signature, ComposeState tracks active_signature_id + signature_separator_index)
- App handlers now use core CRUD (db_insert_signature, db_update_signature, db_delete_signature) instead of raw SQL
- Signature edit detection flag (`dirty: bool` on `SignatureEditorState`) ✅

## Remaining

### Draft restoration with signature state
Draft save does not persist `signature_separator_index` or `active_signature_id`. On draft reopen, signature position in the document is not reconstructed. Tracked in TODO.md.

### Per-account default signature dropdown in Account Settings
Account editor overlay has no signature dropdown for selecting the default signature for an account. Tracked in TODO.md.

## Not a discrepancy

### SignatureEditorMessage flattened into SettingsMessage
Spec proposed a dedicated enum. Code puts editor messages directly in SettingsMessage. Functionally equivalent.
