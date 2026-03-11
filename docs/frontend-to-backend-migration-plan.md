# Frontend to Backend Migration Plan

## Goal

Continue moving business logic out of the TypeScript/Tauri frontend and into the Rust backend while keeping the current frontend functional.

The long-term target is a native frontend. That means the Rust side should become the canonical owner of:

- account creation and lifecycle
- auth and token refresh
- secret encryption/decryption
- provider operations
- sync orchestration
- provider-specific normalization and semantics

The frontend should shrink toward:

- presentation
- local UI state
- event subscriptions
- optimistic UX behavior where needed

## Non-goals

This plan does not attempt to:

- replace the current React frontend now
- redesign user-visible flows
- move all AI/UI-related logic immediately
- rewrite working Rust sync engines just for consistency

## Current State Summary

A large part of the app has already moved to Rust:

- DB reads/writes are already heavily Rust-backed through `invoke()`
- Gmail, JMAP, Graph, and IMAP sync engines largely run in Rust
- provider actions like archive/trash/star/send/drafts mostly already route through Rust provider commands
- attachment fetching already has a Rust-side cache and inline image store

The main remaining TypeScript business-logic clusters are:

1. account/auth/secret lifecycle
2. IMAP provider semantics
3. Gmail/JMAP/Graph adapter normalization
4. sync orchestration and recovery policy
5. post-sync automations
6. folder/label management

There is also an important transition detail in the current architecture:

- TypeScript currently encrypts secrets before writing accounts to the DB.
- Rust provider clients then decrypt those secrets again when they initialize or perform operations.

That means there is a dual encryption/orchestration layer today. The cleanup target is not "add Rust crypto", but "collapse encrypt/decrypt and auth orchestration to Rust as the single owner".

## Target Ownership Boundary

### Rust should own

- account creation, update, deletion, reauthorization
- token exchange, refresh, and provider-specific auth rules
- encryption/decryption of secrets
- provider routing and provider operation semantics
- folder and label CRUD semantics
- message/raw-message/provider-profile fetches
- sync scheduling and fallback policy
- provider-specific DTO normalization
- attachment cache and inline image storage

### TypeScript should own

- React components and routes
- Zustand stores for UI state
- rendering and interaction logic
- toasts, dialogs, progress bars
- optimistic UI updates
- subscribing to backend events and displaying results

## Existing Gaps

The Rust provider abstraction already covers:

- sync
- thread actions
- drafts
- attachments
- folder listing

But it does not yet cover:

- provider test connection
- provider get profile
- provider fetch message
- provider fetch raw message
- provider create folder
- provider rename folder
- provider delete folder

Because of that, TypeScript still needs provider-specific classes and logic.

## Phase 1: Move Account/Auth Ownership to Rust

### Objective

Remove TypeScript ownership of:

- OAuth flow orchestration
- token exchange and refresh
- Gmail client initialization policy
- secret encryption/decryption
- account creation payload construction

More precisely, this phase should collapse the current split where:

- TS orchestrates OAuth and encrypts account secrets before DB writes
- Rust later decrypts the same secrets to use them

### Current TypeScript hot spots

- `src/services/gmail/auth.ts`
- `src/services/oauth/oauthFlow.ts`
- `src/services/gmail/tokenManager.ts`
- `src/services/db/accounts.ts`
- `src/components/accounts/AddAccount.tsx`
- `src/components/accounts/AddImapAccount.tsx`

### Rust commands to add

- `account_begin_oauth`
- `account_complete_oauth`
- `account_create_gmail`
- `account_create_imap`
- `account_create_imap_oauth`
- `account_reauthorize`
- `account_refresh_token_if_needed`
- `account_initialize_clients`

The exact split between `begin/complete` and a single command can vary, but Rust should become the owner of the flow state and token persistence.

### Result

The frontend keeps its forms and UI, but submits typed requests to Rust instead of:

- generating auth state
- exchanging tokens itself
- fetching profile data itself
- encrypting account secrets itself

### Important sequencing constraint

Phase 1 cannot fully stop returning decrypted secrets to TypeScript yet.

That is currently blocked by IMAP ad-hoc operations in the frontend, where TS still:

- reads decrypted account secrets
- builds IMAP/SMTP configs
- refreshes OAuth tokens before IMAP operations

Those dependencies are removed in Phase 3. Until then, some decrypted secret access may still need to remain available to TS for compatibility.

## Phase 2: Expand the Unified Provider API

### Objective

Replace the remaining TypeScript provider classes with thin invoke-backed adapters or remove them entirely.

This phase is primarily unification, not greenfield feature work.

### Extend `ProviderOps`

Add support for:

- `test_connection`
- `get_profile`
- `fetch_message`
- `fetch_raw_message`
- `create_folder`
- `rename_folder`
- `delete_folder`

### Actual current status

Most of these capabilities already exist per-provider, but not behind the unified provider abstraction:

- Gmail already has:
  - test connection
  - label create/update/delete
  - raw message fetch via `gmail_get_message(format=raw)`
- JMAP already has:
  - test connection
  - get profile
  - folder create/update/delete
- Graph already has:
  - test connection
  - get profile
  - Rust-side provider routing for most actions
- IMAP already has:
  - raw message fetch
  - IMAP test connection
  - SMTP test connection

So the main work here is:

- unify existing per-provider commands behind provider-agnostic commands
- add missing IMAP folder CRUD if desired
- remove TS provider-specific wrappers where they are now mostly invoke glue

### Add matching Tauri commands

- `provider_test_connection`
- `provider_get_profile`
- `provider_fetch_message`
- `provider_fetch_raw_message`
- `provider_create_folder`
- `provider_rename_folder`
- `provider_delete_folder`

### Current TypeScript hot spots

- `src/services/email/gmailProvider.ts`
- `src/services/email/imapSmtpProvider.ts`
- `src/services/email/jmapProvider.ts`
- `src/services/email/providerFactory.ts`
- `src/stores/labelStore.ts`

### Result

Provider-specific TS adapters become trivial or disappear. The frontend uses a single backend capability surface.

### Scope note

`gmailProvider.ts` and `jmapProvider.ts` are already close to thin invoke wrappers. They are good early deletion candidates once unified provider commands exist for:

- test connection
- get profile
- folder CRUD

## Phase 3: Move IMAP Semantics Fully into Rust

### Objective

Eliminate the remaining TS-owned IMAP mail semantics.

### Current TypeScript responsibilities to move

- IMAP/SMTP config construction from account rows
- OAuth pre-refresh before IMAP actions
- IMAP folder normalization
- mapping IMAP folders into app label/folder concepts
- parsing synthetic IMAP message IDs
- grouping message IDs into folder/UID batches
- special-folder resolution
- IMAP send/draft behavior

- IMAP OAuth token refresh before non-sync operations

### Current TypeScript hot spots

- `src/services/email/imapSmtpProvider.ts`
- `src/services/imap/imapConfigBuilder.ts`
- `src/services/imap/folderMapper.ts`
- `src/services/imap/messageHelper.ts`
- parts of `src/services/gmail/syncManager.ts`

### Result

IMAP should behave like Gmail/JMAP/Graph from the frontend’s perspective: one invoke layer, no provider-specific business rules in TS.

### Critical prerequisite

Before collapsing `imapSmtpProvider.ts`, verify that Rust-side IMAP operations refresh OAuth access tokens for all ad-hoc operations, not only for sync.

The migration is incomplete if Rust only refreshes tokens during sync but not during:

- fetch attachment
- fetch raw message
- move/delete/archive actions
- send/draft operations
- test connection

## Phase 4: Move Gmail/JMAP/Graph Normalization Behind ProviderOps

### Objective

Make every mail provider return the same frontend-facing DTOs from Rust.

This phase is smaller than it may sound.

### Current TypeScript responsibilities to move

- Gmail label-to-folder shaping
- Gmail raw message decoding
- JMAP folder DTO mapping
- profile/result normalization differences between providers

### Current TypeScript hot spots

- `src/services/email/gmailProvider.ts`
- `src/services/email/jmapProvider.ts`
- `src/stores/labelStore.ts`

### Result

The frontend no longer needs to know how Gmail differs from IMAP or JMAP at the data-shaping level.

### Scope note

Graph is already largely on the target architecture and should not be the main focus of this phase.

The primary beneficiaries are Gmail, IMAP, and JMAP.

## Phase 5: Move Sync Orchestration to Rust

### Objective

Push control flow around sync into the backend.

### Current TypeScript responsibilities to move

- background sync timer
- queueing and deduping sync requests
- initial vs delta decision logic
- recovery fallback policy
- account iteration
- pre-sync token refresh policy
- progress event remapping
- non-blocking follow-up calendar sync coordination

### Current TypeScript hot spot

- `src/services/gmail/syncManager.ts`

### Rust commands/events to add

- `sync_start_background`
- `sync_stop_background`
- `sync_trigger_accounts`
- `sync_force_full`
- `sync_resync_account`
- progress events
- completion/error events

### Result

The frontend subscribes to sync progress and renders it, but no longer owns the orchestration.

### Complexity note

This is one of the highest-risk phases. `syncManager.ts` currently does more than just scheduling:

- queue merging
- initial/delta fallback handling
- post-sync hooks pipeline
- calendar follow-up sync
- event phase translation for UI

This phase is not truly complete without Phase 6.

## Phase 6: Review Post-sync Automations

### Objective

Decide which post-sync behaviors should move to Rust and which should remain UI-side.

### Current behaviors

- filters
- smart labels
- notification eligibility checks
- AI categorization triggers

### Recommendation

Move earlier:

- filter triggering
- smart-label triggering

Likely keep in TS for now:

- actual desktop notification display
- AI provider integrations unless AI is also being moved backend-side

Recommended compromise:

- Rust emits post-sync result events and/or notification candidates
- TS remains responsible for actual desktop notifications
- AI categorization stays TS-side for now

### Current TypeScript hot spot

- `src/services/gmail/syncManager.ts`

## Phase 7: Folder and Label Management Cleanup

### Objective

Remove Gmail-specific folder/label CRUD logic from frontend stores and make folder operations provider-agnostic.

### Current TypeScript hot spot

- `src/stores/labelStore.ts`

### Result

Label/folder UI can remain unchanged while the backend becomes the sole owner of provider-specific folder semantics.

## Status as of 2026-03-11

### Largely complete

- Phase 1: account/auth ownership
- Phase 2: unified provider API
- Phase 3: IMAP semantics migration
- Phase 4: provider normalization cleanup
- Phase 5: sync orchestration migration
- Phase 7: folder/label cleanup

### Partially complete

- Phase 6: post-sync automations

Rust now owns most of:

- account creation and reauthorization flows
- token refresh and secret decryption on read
- provider test/profile/fetch/folder CRUD
- IMAP config building and OAuth freshness
- sync queueing, timers, selection, fallback, and reset prep
- post-sync filters
- criteria-based smart labels
- notification eligibility
- AI categorization candidate selection
- smart-label AI candidate preparation
- calendar provider resolution and persistence
- Google Calendar and CalDAV provider networking

TypeScript still owns a much smaller set of business logic:

- actual AI inference calls and prompt/result shaping
- some settings-loading orchestration
- a few rich account-detail editor flows
- desktop notification display

## Concrete Remaining Work

### 1. Replace full account-row reads used only for UI/editor metadata

Current remaining hotspots:

- `src/components/settings/SettingsAccountsTab.tsx`

This file still loads full `DbAccount` rows because the inline CalDAV editor uses richer account fields than the current basic-info DTO exposes.

Concrete next step:

- add a Rust-backed account details DTO specifically for settings/editor use
- move `SettingsAccountsTab` off `getAccount()` where possible

### 2. Add a Rust-backed settings snapshot for Settings UI bootstrap

Current hotspot:

- `src/components/settings/SettingsPage.tsx`

The settings page still performs many individual `getSetting()` / `getSecureSetting()` calls from TS during initial load.

Concrete next step:

- add one Rust command returning a typed settings snapshot for:
  - Google/Microsoft OAuth settings
  - AI settings
  - notification settings
  - sync settings
  - UI preference defaults that are currently fetched one-by-one
- keep TS-side writes unchanged initially

This is mostly cleanup, but it also moves more "application config shape" knowledge into Rust.

### 3. Decide whether actual AI inference calls should move to Rust

Current state:

- provider/runtime/config selection is already Rust-backed
- TS still assembles prompts and invokes AI for:
  - summaries
  - smart replies
  - compose/reply transforms
  - ask inbox
  - task extraction
  - writing style analysis
  - auto-draft generation
  - smart-label AI classification
  - category inference

Concrete options:

- keep inference calls in TS, with Rust remaining the owner of provider/runtime/config
- or move prompt execution into Rust via typed AI task commands

Suggested next step:

- do not move this blindly
- first decide whether the native-frontend target wants Rust to own:
  - prompt templates
  - output parsing/validation
  - AI task-specific DTOs

If yes, define explicit commands such as:

- `ai_summarize_thread`
- `ai_generate_smart_replies`
- `ai_transform_text`
- `ai_extract_task`
- `ai_generate_auto_draft`

### 4. Finish post-sync automation boundary

Current TS-owned remainder:

- actual desktop notification display
- actual AI smart-label inference call
- actual AI categorization inference call

Current hotspot:

- `src/services/gmail/syncManager.ts`

Concrete next step:

- decide whether `syncManager.ts` should remain a thin event subscriber permanently
- if so, keep:
  - notification display
  - UI progress shaping
- and move:
  - any remaining policy decisions still embedded in TS

### 5. Trim legacy account/settings wrappers that are now mostly compatibility code

Current likely targets:

- `src/services/db/accounts.ts`
- `src/services/db/settings.ts`
- parts of `src/services/gmail/tokenManager.ts`

Concrete next step:

- audit which exports are still used by app code versus only tests
- replace generic DB-shaped helpers with narrower Rust-backed app-facing helpers

### 6. Strengthen regression coverage around migrated sync behavior

Current gap:

- architecture moved faster than broad regression coverage
- targeted tests were updated, but end-to-end sync coverage remains thinner than ideal

Concrete next step:

- repair and expand `src/test/syncManager.test.ts`
- add focused tests for:
  - sync status event handling
  - background sync start/stop behavior
  - post-sync hook triggering
  - account bootstrap paths that now use account-summary DTOs

## Likely Remaining Files to Shrink

- `src/components/settings/SettingsPage.tsx`
- `src/components/settings/SettingsAccountsTab.tsx`
- `src/services/gmail/syncManager.ts`
- `src/services/db/accounts.ts`
- `src/services/db/settings.ts`
- `src/services/gmail/tokenManager.ts`

## Current Risks

- `SettingsAccountsTab` still depends on richer account data than the new summary DTOs expose.
- The remaining TS-side AI call layer is now the largest intentionally un-migrated business-logic seam.
- Sync behavior is user-visible, and although ownership moved successfully, broader regression coverage should catch up.

## Updated Recommended Execution Order

1. Add Rust-backed settings/account-detail snapshot commands for settings UI
2. Trim remaining TS full-account reads and generic wrappers
3. Decide the final boundary for AI task execution
4. If desired, move AI task execution into Rust with typed commands
5. Expand sync regression coverage and remove any leftover compatibility glue

## Progress Log

- 2026-03-11: Initial migration plan written based on current repo structure and command surface.
- 2026-03-11: Major migration progress completed across account/auth, provider unification, IMAP semantics, sync orchestration, post-sync hooks, calendar providers, and AI runtime.
- 2026-03-11: Plan updated to reflect current status and concrete remaining work instead of the original broad migration phases.
