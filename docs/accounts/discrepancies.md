# Accounts: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21

## What Matches the Spec

### Phase 0: Data Model Extensions
- **DB migration**: `account_color TEXT`, `account_name TEXT`, `sort_order INTEGER` columns added in `crates/db/src/db/migrations.rs` (lines 1322-1324). SMTP username/password columns also added (lines 1336-1337). Matches spec.
- **App-side `Account` type** (`crates/app/src/db/types.rs`): Has `account_name`, `account_color`, `sort_order`, `last_sync_at` fields. Matches spec.
- **Core CRUD functions** (`crates/core/src/db/queries_extra/accounts_crud.rs`): All six spec'd functions implemented: `db_create_account`, `db_update_account`, `db_update_account_color`, `db_update_account_name`, `db_update_account_sort_order`, `db_account_exists_by_email`. `CreateAccountParams` and `UpdateAccountParams` structs match spec. Sort order auto-increments on creation.
- **Query ordering**: Account loading query uses `ORDER BY sort_order ASC, created_at ASC` as spec'd.

### Phase 1: First Launch Detection & Empty State
- **`no_accounts` flag** on `App` struct. Set in `handle_accounts_loaded` when accounts list is empty. Matches spec.
- **`add_account_wizard` field** on `App`. Auto-opened via `AddAccountWizard::new_first_launch()` when no accounts. Matches spec.
- **View routing**: `view_main_window()` checks `add_account_wizard` and `no_accounts` to render first-launch modal vs. overlay modal vs. normal layout. Matches spec.
- **First-launch modal**: `view_first_launch_modal()` centers the wizard in the window using `ACCOUNT_MODAL_WIDTH` (520.0) and `ACCOUNT_MODAL_MAX_HEIGHT` (640.0). Matches spec layout constants.

### Phase 2: Add Account Wizard State Machine
- **`AddAccountStep` enum**: All nine variants present (`EmailInput`, `Discovering`, `ProtocolSelection`, `ManualConfiguration`, `OAuthWaiting`, `PasswordAuth`, `Validating`, `Identity`, `Creating`). Matches spec.
- **`AddAccountWizard` struct**: Has `step`, `is_first_launch`, `email`, `error`, `generation`, `auth_state`, `identity`, `used_colors`. Matches spec.
- **`AuthState` struct**: All fields present (username, password, smtp_username, smtp_password, use_separate_smtp_credentials, accept_invalid_certs, imap/smtp host/port/security). Matches spec.
- **`SecurityOption` enum**: `Tls`, `StartTls`, `None` with `label()` and `to_db_string()`. Matches spec.
- **`AddAccountEvent` enum**: `AccountAdded(String)` and `Cancelled`. Matches spec.
- **Component trait**: `AddAccountWizard` implements `Component` with `Message = AddAccountMessage`, `Event = AddAccountEvent`. Matches spec.
- **Generation counter**: Used in `SubmitEmail` and `SubmitIdentity` for staleness detection; `DiscoveryComplete` and `AccountCreated` check generation. Matches spec.
- **Color pre-selection**: First unused color from the 25-preset palette is pre-selected. Matches spec.

### Phase 3: Wizard Views
- **Email input view**: Welcome text for first launch, "Add Account" for subsequent. Email text input with placeholder, Continue button, Cancel (non-first-launch only). Matches spec.
- **Discovering view**: "Looking up your email provider..." with cancel button. Matches spec.
- **Password auth view**: IMAP server/port, security selector, username, password (plaintext -- explicitly commented as intentional per spec), SMTP server/port/security, separate SMTP credentials toggle, accept self-signed certs checkbox. Matches spec.
- **Identity view**: Email display, account name input with titlecase pre-fill from domain, color palette grid with 25 swatches (5 columns, 28px), used-color dimming. Matches spec.
- **OAuth waiting view**: "Complete sign-in in your browser", "Waiting for authorization...", error display, Cancel button. Matches spec structure.
- **Back navigation**: Correct back transitions (PasswordAuth/ManualConfig -> EmailInput, Identity -> PasswordAuth, ProtocolSelection -> EmailInput). Matches spec.

### Phase 4: App-Level Message Wiring
- **`Message::AddAccount(AddAccountMessage)`** and **`Message::OpenAddAccount`** variants. Matches spec.
- **`handle_add_account()`**: Delegates to wizard, handles events. Matches spec.
- **`handle_add_account_event()`**: On `AccountAdded`, closes wizard, sets `no_accounts = false`, reloads accounts. On `Cancelled`, only closes if accounts exist. Matches spec.
- **`view_with_add_account_modal()`**: Overlay with blocker + stack pattern. Uses `ModalBackdrop` container style. Matches spec.
- **Settings wiring**: `SettingsEvent::OpenAddAccountWizard` triggers wizard open from settings. Matches spec.

### Phase 5: Account Management in Settings (Partial)
- **`Tab::Accounts`** is the first tab in `Tab::ALL`. Label is "Accounts", icon is `users()`. Matches spec.
- **`ManagedAccount` struct**: Has `id`, `email`, `provider`, `account_name`, `account_color`, `display_name`, `last_sync_at`. Matches spec (minus `health`).
- **Accounts tab view**: Lists account cards + "Add Account" button. Matches spec layout.
- **Account card**: Shows color dot (when available), name fallback chain (account_name -> display_name -> email), provider label, sync time. Matches spec visually.
- **`format_last_sync()`**: Relative timestamps ("Just now", "X minutes ago", etc.). Matches spec.
- **`managed_accounts` sync**: `handle_accounts_loaded()` populates `settings.managed_accounts` from account data. Matches spec.

### Layout Constants
- `ACCOUNT_MODAL_WIDTH: 520.0`, `ACCOUNT_MODAL_MAX_HEIGHT: 640.0`, `COLOR_SWATCH_SIZE: 28.0`, `COLOR_PALETTE_COLUMNS: 5`, `PROTOCOL_CARD_HEIGHT: 64.0`. All present in `crates/app/src/ui/layout.rs`. Matches spec.

### Theme Additions
- `ContainerClass::ModalBackdrop` exists. Matches spec.

---

## What Diverges from the Spec

### Discovery is a stub
The spec calls for wiring `ratatoskr_core::discovery::discover()` with a 15-second timeout, duplicate-account check via `db_account_exists_by_email`, and auto-proceed logic based on `source.is_high_confidence()`. The implementation has a TODO placeholder that immediately returns `Ok(())` without calling the real discovery backend. The `DiscoveryComplete` message carries `Result<(), String>` instead of `Result<DiscoveredConfig, String>` -- the `DiscoveredConfig` type is not used at all. This means:
- No duplicate account check before discovery.
- No auto-proceed vs. protocol-selection branching.
- Discovery always "succeeds" and falls through to password auth.

### Protocol selection is a placeholder
The spec defines `SelectProtocol(usize)`, `ConfirmProtocol`, protocol cards with `protocol_card()` widget, and `ProtocolOption`/`DiscoveredConfig` integration. The implementation's `view_protocol_selection()` renders static text ("Protocol selection coming soon.") with no interactive cards. The `AddAccountMessage` enum is missing `SelectProtocol`, `ConfirmProtocol`. The wizard has no `discovery: Option<DiscoveredConfig>` or `selected_option: Option<usize>` fields.

### OAuth flow is not wired
The spec defines `OAuthComplete(u64, Result<OAuthSuccess, String>)`, `RetryOAuth`, and the full OAuth browser handoff flow with `authorize_with_provider()`. The implementation has the `OAuthWaiting` step and view, but no `OAuthComplete` or `RetryOAuth` messages. The OAuth waiting view shows a cancel button but no retry on error. No OAuth task is ever spawned.

### Manual configuration is simplified
The spec defines `ManualConfig` struct with `selected_provider: Option<ManualProvider>`, `ManualProvider` enum (Gmail, Microsoft365, Jmap, Imap), `ManualAuthMethod` enum, security selectors, and JMAP URL field. The implementation reuses `AuthState` fields directly for manual config (IMAP host/port, SMTP host/port only). No provider type selection cards, no JMAP URL, no auth method selector, no security options in manual config form.

### Credential validation is skipped
The spec defines a `Validating` step with `ValidationComplete(u64, Result<(), String>)` message for testing IMAP connections. The implementation has the `Validating` step enum variant and a static view, but `handle_submit_credentials()` skips validation entirely and jumps straight to `Identity`. No `ValidationComplete` message exists.

### Core CRUD bypassed for account creation
**Cross-cutting concern (f).** The spec calls for using `db_create_account()` from the core CRUD module. The implementation writes raw SQL via `db.with_write_conn()` directly in `add_account.rs` (lines 398-458). This duplicates the INSERT logic and misses the `sort_order` auto-increment that `db_create_account` provides. The raw SQL also hardcodes `provider = 'imap'` and `auth_method = 'password'`, ignoring OAuth providers entirely.

### Sidebar dropdown does not use account_name or color
The spec (Phase 6) calls for showing `account_name` instead of email and using `DropdownIcon::ColorDot(Color)` for account color indicators. The implementation uses `display_name` (not `account_name`) and `DropdownIcon::Avatar(name)` -- there is no `ColorDot` variant on `DropdownIcon`. The spec noted this as a TODO in a comment.

### Color swatch uses `Chip` button style, not `ColorSwatchSelected`
The spec defines `ButtonClass::ColorSwatchSelected` for the selected swatch ring. The implementation uses `ButtonClass::Chip { active: true }` for selected swatches and `ButtonClass::BareTransparent` for unselected. The `ColorSwatchSelected` variant does not exist in the theme.

### Missing `ProtocolCard`/`ProtocolCardSelected` button styles
The spec defines `ButtonClass::ProtocolCard` and `ButtonClass::ProtocolCardSelected`. Neither exists in the theme since protocol selection is a placeholder.

---

## What Is Missing

### Account editor slide-in (Phase 5b)
The spec defines `AccountEditor` struct, `SettingsOverlay::AccountEditor`, and a full slide-in editor with editable account name, display name, color picker, CalDAV settings, re-authenticate action, and delete with confirmation. None of this is implemented:
- No `AccountEditor` struct.
- No `SettingsOverlay::AccountEditor` variant.
- No `AccountCardClicked` message handler (the card is not clickable -- `TODO` comment at line 1586 of `tabs.rs`).
- No messages: `CloseAccountEditor`, `SaveAccountEditor`, `AccountNameEditorChanged`, `DisplayNameEditorChanged`, `AccountColorEditorChanged`, `CaldavUrlChanged/UsernameChanged/PasswordChanged`, `ReauthenticateAccount`, `DeleteAccountRequested`, `DeleteAccountConfirmed`, `DeleteAccountCancelled`.

### Account deletion
No delete account functionality exists. No `SettingsEvent::DeleteAccount`, no `Message::AccountDeleted`, no `handle_account_deleted()`, no `db_delete_account` core function. The edge cases (last account -> first-launch state, revert account selector, cancel active sync) are all unimplemented.

### Account reordering via drag
No drag-to-reorder for account cards. The spec defines `AccountDragGripPress`, `AccountDragMove`, `AccountDragEnd` messages and `SettingsEvent::AccountsReordered`. None exist. The `DragState` infrastructure exists for editable lists (Mail Rules tab) but is not wired to account cards.

### AccountHealth type and health indicators
The spec defines `AccountHealth` enum (`Healthy`, `Warning`, `Error`, `Disabled`), `compute_health()`, and a health dot in account cards. None of this exists:
- No `AccountHealth` enum anywhere in the codebase.
- `ManagedAccount` has no `health` field.
- Account cards show no health indicator.
- `Account` type in `db/types.rs` lacks `token_expires_at` and `is_active` fields needed for health derivation.

### Re-authentication flow (Phase 7)
The spec defines `ReauthWizard` state with `account_id`, `account_email`, `generation`, `error`. No such struct or flow exists. No `Message::ReauthenticateAccount` in the app. The status bar has `AccountWarning` / `WarningKind::TokenExpiry` types but no wiring to trigger re-auth.

### Duplicate account detection
The spec calls for `db_account_exists_by_email` check during email submission (before discovery). The core function exists but is never called from the app. Users can potentially add the same email twice.

### Missing spec'd messages
These `AddAccountMessage` variants from the spec are absent:
- `SelectProtocol(usize)`
- `ConfirmProtocol`
- `OAuthComplete(u64, Result<OAuthSuccess, String>)`
- `RetryOAuth`
- `ValidationComplete(u64, Result<(), String>)`
- `DismissError`
- Manual config messages: `ManualImapSecurityChanged`, `ManualSmtpSecurityChanged`, `ManualJmapUrlChanged`, `ManualAuthMethodChanged`, `SelectManualProvider`

### `OAuthSuccess` type
The spec defines an `OAuthSuccess` struct with token fields, user info, and provider metadata. Not implemented.

---

## Cross-Cutting Concern Status

### (a) Generational load tracking
**Partially implemented.** The `AddAccountWizard` uses a `generation: u64` counter. It is incremented on `SubmitEmail` and `SubmitIdentity`, and stale results are discarded for `DiscoveryComplete` and `AccountCreated`. However, since discovery is a stub and OAuth/validation are not wired, the generation tracking for those async tasks (`OAuthComplete`, `ValidationComplete`) is absent. The `App`-level `nav_generation` is used for account reloads. The pattern is correct where applied.

### (b) Component trait
**Implemented.** Both `AddAccountWizard` and `Settings` implement the `Component` trait from `crates/app/src/component.rs` with proper `Message`/`Event` associated types, `update()` returning `(Task, Option<Event>)`, and `view()`. The `App` delegates to components and handles events. The pattern is clean and consistent.

### (c) Token-to-Catalog theming
**Mostly implemented.** The wizard views use named style classes (`theme::TextInputClass::Settings.style()`, `theme::ButtonClass::Action.style()`, `theme::ButtonClass::BareTransparent.style()`, `theme::ContainerClass::ModalBackdrop.style()`, etc.). The color swatch uses `ButtonClass::Chip { active: true }` instead of the spec'd `ColorSwatchSelected`. No inline style closures observed. However, the spec'd `ProtocolCard`/`ProtocolCardSelected` classes are missing because protocol selection is not implemented.

### (d) iced_drop drag-and-drop
**Not implemented** for accounts. No `iced_drop` or `Droppable` usage anywhere in the app crate. Account reordering via drag is not implemented. The existing editable-list drag uses `mouse_area`/`DragState`, not `iced_drop`.

### (e) Subscription orchestration
**Not applicable.** Neither `AddAccountWizard` nor the accounts portion of `Settings` defines a `subscription()` override. The default `Subscription::none()` is used. The spec does not explicitly require subscriptions for accounts (the status bar subscription for health monitoring is a separate concern in Phase 7, which is not implemented).

### (f) Core CRUD bypassed
**Bypassed.** The `handle_submit_identity()` function writes a raw `INSERT INTO accounts` SQL statement via `db.with_write_conn()` instead of calling `db_create_account()` from the core CRUD module. This is a direct violation of the spec and the crate architecture principle that business logic belongs in `ratatoskr-core`. The raw SQL also misses `sort_order` auto-increment and only supports IMAP/password accounts.

### (g) Dead code
- **`AddAccountStep::ProtocolSelection`**: The enum variant exists and has a view function, but the step is never reached in the normal flow (discovery stub always succeeds -> goes to PasswordAuth). The view is a static placeholder.
- **`AddAccountStep::Validating`**: The enum variant exists and has a view function (`view_validating()`), but the step is never reached (`handle_submit_credentials` skips to `Identity`).
- **`PROTOCOL_CARD_HEIGHT`**: Layout constant defined (64.0) but unused since protocol cards are not rendered.
- **Catch-all `_ => {}` in `handle_field_update`**: Silently drops unrecognized messages. Not technically dead code, but could mask bugs if new message variants are added without handler arms.
- **`ManualImapHostChanged`/`ManualImapPortChanged`/`ManualSmtpHostChanged`/`ManualSmtpPortChanged`**: These messages duplicate the `AuthImapHostChanged`/etc. functionality -- both sets write to the same `auth_state` fields. The manual config messages could be removed in favor of reusing the auth messages, or the manual config could have its own state as the spec intended.
