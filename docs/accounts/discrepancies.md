# Accounts: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21 (updated after implementation pass)

## What Matches the Spec

### Phase 0: Data Model Extensions
- **DB migration**: `account_color TEXT`, `account_name TEXT`, `sort_order INTEGER` columns added in `crates/db/src/db/migrations.rs` (lines 1322-1324). SMTP username/password columns also added (lines 1336-1337). Matches spec.
- **App-side `Account` type** (`crates/app/src/db/types.rs`): Has `account_name`, `account_color`, `sort_order`, `last_sync_at` fields. Matches spec.
- **Core CRUD functions** (`crates/core/src/db/queries_extra/accounts_crud.rs`): All six spec'd functions implemented: `db_create_account`, `db_update_account`, `db_update_account_color`, `db_update_account_name`, `db_update_account_sort_order`, `db_account_exists_by_email`. `CreateAccountParams` and `UpdateAccountParams` structs match spec. Sort order auto-increments on creation. **NEW:** Synchronous variants `create_account_sync` and `account_exists_by_email_sync` added for use from the app crate's `Db::with_write_conn`.
- **Query ordering**: Account loading query uses `ORDER BY sort_order ASC, created_at ASC` as spec'd.

### Phase 1: First Launch Detection & Empty State
- **`no_accounts` flag** on `App` struct. Set in `handle_accounts_loaded` when accounts list is empty. Matches spec.
- **`add_account_wizard` field** on `App`. Auto-opened via `AddAccountWizard::new_first_launch()` when no accounts. Matches spec.
- **View routing**: `view_main_window()` checks `add_account_wizard` and `no_accounts` to render first-launch modal vs. overlay modal vs. normal layout. Matches spec.
- **First-launch modal**: `view_first_launch_modal()` centers the wizard in the window using `ACCOUNT_MODAL_WIDTH` (520.0) and `ACCOUNT_MODAL_MAX_HEIGHT` (640.0). Matches spec layout constants.

### Phase 2: Add Account Wizard State Machine
- **`AddAccountStep` enum**: All nine variants present (`EmailInput`, `Discovering`, `ProtocolSelection`, `ManualConfiguration`, `OAuthWaiting`, `PasswordAuth`, `Validating`, `Identity`, `Creating`). Matches spec.
- **`AddAccountWizard` struct**: Has `step`, `is_first_launch`, `email`, `error`, `generation`, `auth_state`, `identity`, `used_colors`, **`discovery`**, **`selected_option`**, **`oauth_success`**, **`resolved_provider`**, **`resolved_auth_method`**. Matches spec.
- **`AuthState` struct**: All fields present (username, password, smtp_username, smtp_password, use_separate_smtp_credentials, accept_invalid_certs, imap/smtp host/port/security). Matches spec.
- **`SecurityOption` enum**: `Tls`, `StartTls`, `None` with `label()` and `to_db_string()`. Matches spec.
- **`AddAccountEvent` enum**: `AccountAdded(String)` and `Cancelled`. Matches spec.
- **Component trait**: `AddAccountWizard` implements `Component` with `Message = AddAccountMessage`, `Event = AddAccountEvent`. Matches spec.
- **Generation counter**: Used in `SubmitEmail`, `SubmitCredentials`, `SubmitIdentity`, and OAuth flow for staleness detection. `DiscoveryComplete`, `OAuthComplete`, `ValidationComplete`, and `AccountCreated` all check generation. Matches spec.
- **Color pre-selection**: First unused color from the 25-preset palette is pre-selected. Matches spec.
- **`OAuthSuccess` type**: Implemented with token fields, user info, and provider metadata. Matches spec.

### Phase 3: Wizard Views
- **Email input view**: Welcome text for first launch, "Add Account" for subsequent. Email text input with placeholder, Continue button, Cancel (non-first-launch only). Matches spec.
- **Discovering view**: "Looking up your email provider..." with cancel button. Matches spec.
- **Protocol selection view**: **IMPLEMENTED.** Shows discovered protocol options as selectable cards with provider name, detail, and source label. Pre-selects top option. Continue/Back buttons. Matches spec.
- **Password auth view**: IMAP server/port, security selector, username, password (plaintext -- explicitly commented as intentional per spec), SMTP server/port/security, separate SMTP credentials toggle, accept self-signed certs checkbox. Matches spec.
- **OAuth waiting view**: "Complete sign-in in your browser", "Waiting for authorization...", error display with Retry button, Cancel button. Matches spec.
- **Identity view**: Email display, account name input with titlecase pre-fill from domain, color palette grid with 25 swatches (5 columns, 28px), used-color dimming. Matches spec.
- **Back navigation**: Correct back transitions (PasswordAuth/ManualConfig -> EmailInput, Identity -> PasswordAuth or OAuthWaiting, ProtocolSelection -> EmailInput). Matches spec.

### Phase 4: App-Level Message Wiring
- **`Message::AddAccount(AddAccountMessage)`**, **`Message::OpenAddAccount`**, **`Message::AccountDeleted`**, **`Message::AccountUpdated`** variants. Matches spec.
- **`handle_add_account()`**: Delegates to wizard, handles events. Matches spec.
- **`handle_add_account_event()`**: On `AccountAdded`, closes wizard, sets `no_accounts = false`, reloads accounts. On `Cancelled`, only closes if accounts exist. Matches spec.
- **`view_with_add_account_modal()`**: Overlay with blocker + stack pattern. Uses `ModalBackdrop` container style. Matches spec.
- **Settings wiring**: `SettingsEvent::OpenAddAccountWizard` triggers wizard open from settings. Matches spec.

### Phase 5: Account Management in Settings
- **`Tab::Accounts`** is the first tab in `Tab::ALL`. Label is "Accounts", icon is `users()`. Matches spec.
- **`ManagedAccount` struct**: Has `id`, `email`, `provider`, `account_name`, `account_color`, `display_name`, `last_sync_at`, **`health`**. Matches spec.
- **`AccountHealth` enum**: **IMPLEMENTED.** `Healthy`, `Warning`, `Error`, `Disabled` variants with `compute_health()` function. Matches spec.
- **Health indicator in account cards**: **IMPLEMENTED.** Small colored dot showing health status. Matches spec.
- **Accounts tab view**: Lists account cards + "Add Account" button. Matches spec layout.
- **Account card**: **CLICKABLE.** Shows color dot, name fallback chain, provider label, sync time, health dot, chevron. Clicking opens account editor. Matches spec.
- **`format_last_sync()`**: Relative timestamps ("Just now", "X minutes ago", etc.). Matches spec.
- **`managed_accounts` sync**: `handle_accounts_loaded()` populates `settings.managed_accounts` from account data with health computation. Matches spec.
- **`AccountEditor` struct**: **IMPLEMENTED.** Has `account_id`, `account_email`, `account_name`, `display_name`, `account_color_index`, `caldav_url/username/password`, `show_delete_confirmation`, `dirty`. Matches spec.
- **`SettingsOverlay::AccountEditor`**: **IMPLEMENTED.** Slide-in editor overlay with name, color picker, CalDAV settings, re-auth action, delete with confirmation. Matches spec.
- **Account deletion**: **IMPLEMENTED.** `DeleteAccountRequested` shows confirmation, `DeleteAccountConfirmed` executes deletion, `DeleteAccountCancelled` hides confirmation. `SettingsEvent::DeleteAccount` propagates to App. App handles last-account edge case (reloads accounts -> detects empty -> shows first-launch modal). Matches spec.
- **Account update**: **IMPLEMENTED.** `SaveAccountEditor` emits `SettingsEvent::SaveAccountChanges` with `UpdateAccountParams`. App persists changes and reloads accounts. Matches spec.

### Phase 7 (partial): Error States & Recovery
- **Duplicate account detection**: **IMPLEMENTED.** `account_exists_by_email_sync` is called during email submission (before discovery). Error displayed on EmailInput step. Matches spec.

### Layout Constants
- `ACCOUNT_MODAL_WIDTH: 520.0`, `ACCOUNT_MODAL_MAX_HEIGHT: 640.0`, `COLOR_SWATCH_SIZE: 28.0`, `COLOR_PALETTE_COLUMNS: 5`, `PROTOCOL_CARD_HEIGHT: 64.0`. All present in `crates/app/src/ui/layout.rs`. Matches spec.

### Theme Additions
- `ContainerClass::ModalBackdrop` exists. Matches spec.

### Core CRUD wired for account creation
- **Account creation uses `create_account_sync`**: The wizard's `handle_submit_identity` now calls the core CRUD function `create_account_sync` inside `Db::with_write_conn`, replacing the raw SQL. Provider and auth_method are set correctly from discovery results (not hardcoded to 'imap'/'password'). Sort order auto-increments. Matches spec.

### Discovery wired
- **Real discovery**: `ratatoskr_core::discovery::discover()` is called with its built-in 15-second timeout. Duplicate account check runs first via `account_exists_by_email_sync`. Discovery results branch on `source.is_high_confidence()` for auto-proceed vs. protocol selection. Matches spec.

### OAuth flow wired
- **`OAuthComplete`** and **`RetryOAuth`** messages implemented. OAuth browser handoff uses `authorize_with_provider()` from core. Browser opened via platform-specific command. Token results stored in `OAuthSuccess` and used in `CreateAccountParams` for account creation. Matches spec.

### Credential validation wired
- **`Validating` step**: `handle_submit_credentials` transitions to `Validating` step. IMAP connection test uses `ratatoskr_core::imap::connection::connect()` with `ImapConfig`. `ValidationComplete` message handles success (-> Identity) or failure (-> PasswordAuth with error). Matches spec.

---

## What Diverges from the Spec

### Manual configuration is simplified
The spec defines `ManualConfig` struct with `selected_provider: Option<ManualProvider>`, `ManualProvider` enum (Gmail, Microsoft365, Jmap, Imap), `ManualAuthMethod` enum, security selectors, and JMAP URL field. The implementation reuses `AuthState` fields directly for manual config (IMAP host/port, SMTP host/port only). No provider type selection cards, no JMAP URL, no auth method selector. This is a minor gap since the manual config path is an escape hatch, not the primary flow.

### Sidebar dropdown does not use account_name or color
The spec (Phase 6) calls for showing `account_name` instead of email and using `DropdownIcon::ColorDot(Color)` for account color indicators. The implementation uses `display_name` (not `account_name`) and `DropdownIcon::Avatar(name)` -- there is no `ColorDot` variant on `DropdownIcon`. This is Phase 6 work and was not in scope for this pass.

### Color swatch uses `Chip` button style, not `ColorSwatchSelected`
The spec defines `ButtonClass::ColorSwatchSelected` for the selected swatch ring. The implementation uses `ButtonClass::Chip { active: true }` for selected swatches and `ButtonClass::BareTransparent` for unselected. The `ColorSwatchSelected` variant does not exist in the theme.

### Missing `ProtocolCard`/`ProtocolCardSelected` button styles
The spec defines `ButtonClass::ProtocolCard` and `ButtonClass::ProtocolCardSelected`. The implementation uses `ButtonClass::Chip { active: true }` for selected and `ButtonClass::Action` for unselected protocol cards. Functionally equivalent but not the exact named styles.

---

## What Is Missing

### Account reordering via drag
No drag-to-reorder for account cards. The spec defines `AccountDragGripPress`, `AccountDragMove`, `AccountDragEnd` messages and `SettingsEvent::AccountsReordered`. None exist. The `DragState` infrastructure exists for editable lists (Mail Rules tab) but is not wired to account cards.

### Re-authentication flow (Phase 7)
The spec defines `ReauthWizard` state with `account_id`, `account_email`, `generation`, `error`. No such struct or flow exists. The `ReauthenticateAccount` message handler in settings is a TODO stub. The status bar has `AccountWarning` / `WarningKind::TokenExpiry` types but no wiring to trigger re-auth.

### `color_palette_grid` not yet reusable in widgets.rs
The spec calls for moving the color palette grid from add_account.rs to widgets.rs with a generic `on_select` callback. The add_account.rs still has its own local `color_palette_grid`, and the account editor overlay builds an inline palette. The widget should be unified into `widgets.rs`.

### Account health lacks `token_expires_at` and `is_active` from DB
`compute_health()` exists but is called with `token_expires_at: None` and `is_active: true` because the app-side `Account` type doesn't carry these fields from the DB yet. Health is always `Healthy` until these fields are plumbed through.

### Phase 6: Sidebar Enhancements
Color dots in the dropdown, `account_name` display, `DropdownIcon::ColorDot` variant -- all deferred.

---

## Cross-Cutting Concern Status

### (a) Generational load tracking
**Fully implemented for accounts.** All async tasks use generation counters: `DiscoveryComplete`, `OAuthComplete`, `ValidationComplete`, `AccountCreated`. Stale results are silently discarded.

### (b) Component trait
**Implemented.** `AddAccountWizard` and `Settings` both implement the `Component` trait.

### (c) Token-to-Catalog theming
**Mostly implemented.** Named style classes used everywhere. Protocol cards use `Chip`/`Action` instead of the spec'd `ProtocolCard`/`ProtocolCardSelected`. No inline style closures.

### (d) iced_drop drag-and-drop
**Not implemented** for accounts. Account reordering via drag is deferred.

### (e) Subscription orchestration
**Not applicable.** No subscriptions needed for the account flows implemented so far.

### (f) Core CRUD wired
**Fixed.** `handle_submit_identity()` now calls `create_account_sync()` from the core CRUD module instead of raw SQL. Provider and auth method are determined from discovery results, not hardcoded. Sort order auto-increments.

### (g) Dead code
- **`ManualImapHostChanged`/`ManualImapPortChanged`/`ManualSmtpHostChanged`/`ManualSmtpPortChanged`**: These messages duplicate the `AuthImapHostChanged`/etc. functionality since both sets write to the same `auth_state` fields. Could be consolidated.
- **Catch-all `_ => {}` in `handle_field_update`**: Silently drops unrecognized messages.
