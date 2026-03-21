# Accounts: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Manual configuration is simplified
Spec defines `ManualConfig` struct with `ManualProvider` enum (Gmail, Microsoft365, Jmap, Imap), `ManualAuthMethod` enum, JMAP URL field. Code reuses `AuthState` fields for manual config (IMAP host/port, SMTP host/port only). No provider type selection, no JMAP URL, no auth method selector.
- Spec: `docs/accounts/implementation-spec.md` Phase 3 ManualConfiguration
- Code: `crates/app/src/ui/add_account.rs:617-627` (`handle_submit_manual_config` hardcodes provider to "imap")

### Button style names diverge from spec
Spec defines `ButtonClass::ColorSwatchSelected`, `ButtonClass::ProtocolCard`, `ButtonClass::ProtocolCardSelected`. Code uses `ButtonClass::Chip { active: true }` and `ButtonClass::Action`/`ButtonClass::BareTransparent` instead. Functionally equivalent but not the named styles.
- Spec: `docs/accounts/implementation-spec.md` Phase 3 theme additions
- Code: `crates/app/src/ui/settings/tabs.rs:731-737` (color swatch), `crates/app/src/ui/add_account.rs` (protocol cards)

### Account reordering via drag not implemented
Spec defines `AccountDragGripPress`, `AccountDragMove`, `AccountDragEnd` messages and `SettingsEvent::AccountsReordered`. None exist. `DragState` infrastructure exists for editable lists but is not wired to account cards.
- Spec: `docs/accounts/implementation-spec.md` Phase 5 reordering section
- Code: `crates/app/src/ui/settings/types.rs:496-506` (DragState exists, not used for accounts)

### Re-authentication flow not implemented (Phase 7)
Spec defines `ReauthWizard` struct with `account_id`, `account_email`, `generation`, `error`. No such struct exists. `ReauthenticateAccount` message handler is a TODO stub that does nothing.
- Spec: `docs/accounts/implementation-spec.md` Phase 7 lines ~1851-1864
- Code: `crates/app/src/ui/settings/update.rs:76-78` (TODO comment, no-op)

### AccountHealth always returns Healthy
`compute_health()` is called with `token_expires_at: None` and `is_active: true` because the app-side `Account` type (`crates/app/src/db/types.rs`) does not carry `token_expires_at` or `is_active` fields. Health indicator renders but is always green.
- Spec: `docs/accounts/implementation-spec.md` lines ~121-146 (Account type must carry `token_expires_at`, `is_active`)
- Code: `crates/app/src/main.rs:1370` (`compute_health(a.last_sync_at, None, true)`)

### Sidebar dropdown does not use account_name or color (Phase 6)
Spec calls for `account_name` display and `DropdownIcon::ColorDot(Color)` in sidebar. Code uses `display_name` and `DropdownIcon::Avatar(name)`. No `ColorDot` variant exists.
- Spec: `docs/accounts/implementation-spec.md` Phase 6 lines ~1793-1820
- Code: sidebar `scope_dropdown()` (not yet updated)

### `color_palette_grid` not unified into widgets.rs
Spec calls for a reusable palette grid in `widgets.rs`. `add_account.rs:1489` has a local `color_palette_grid`; `settings/tabs.rs:710-763` has a separate inline palette. Not shared.
- Spec: `docs/accounts/implementation-spec.md` (widget reuse)
- Code: `crates/app/src/ui/add_account.rs:1489`, `crates/app/src/ui/settings/tabs.rs:710`

### Account update and delete bypass core CRUD
Account creation uses `create_account_sync` from core. However, `handle_save_account_changes` (`crates/app/src/main.rs:1207-1257`) builds raw SQL dynamically, and `handle_delete_account` (`crates/app/src/main.rs:1182-1205`) uses raw `DELETE FROM accounts`. Both should use `db_update_account` and `db_delete_account` from `crates/core/src/db/queries_extra/`.
- Core CRUD: `crates/core/src/db/queries_extra/accounts_crud.rs:145` (`db_update_account`)
- Core delete: `crates/core/src/db/queries_extra/accounts_messages.rs:91` (`db_delete_account`)

### Account deletion does not cascade
Spec says deletion "removes the account and all its data (labels, threads, messages, attachments, cached files)" with cascading. Both app and core delete only run `DELETE FROM accounts WHERE id = ?1` with no explicit cascade logic. Relies entirely on DB foreign key constraints (unverified whether those are configured).
- Spec: `docs/accounts/problem-statement.md` line ~176
- Code: `crates/app/src/main.rs:1194-1195`
