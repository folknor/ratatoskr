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

## Recommended First Implementation Slice

This is the best first chunk to implement without destabilizing the UI:

1. Move account/auth ownership to Rust, including collapsing the dual TS/Rust encryption flow.
2. Add unified provider commands for:
   - `provider_test_connection`
   - `provider_get_profile`
   - `provider_create_folder`
   - `provider_rename_folder`
   - `provider_delete_folder`
3. Update the current frontend to consume those commands.
4. Verify Rust IMAP operations refresh OAuth tokens before ad-hoc operations.

### Why this slice first

- It removes real business logic instead of only wrappers.
- It stabilizes a backend API that a native frontend can also consume later.
- It avoids starting with the riskier sync-orchestration rewrite.
- It should noticeably reduce frontend complexity immediately.

### Explicit deferral

`provider_fetch_raw_message` is useful, but not required in the first slice.

`RawMessageModal.tsx` can keep going through the existing provider adapter until the unified provider surface is further along.

## Concrete First Patch Set

### Rust

- add account/auth command module for Gmail and OAuth IMAP account flows
- extend `ProviderOps`
- add Tauri commands for provider test/profile/folder CRUD
- normalize provider DTOs in Rust

### TypeScript

- replace OAuth/token flow logic in account setup components with invoke-backed calls
- replace Gmail client init logic with one backend call
- replace `labelStore` Gmail-specific commands with provider-agnostic calls
- shrink or remove provider-specific TS classes where possible

## Likely Files to Shrink or Disappear

High probability:

- `src/services/gmail/auth.ts`
- `src/services/oauth/oauthFlow.ts`
- `src/services/gmail/tokenManager.ts`
- `src/services/email/gmailProvider.ts`
- `src/services/email/imapSmtpProvider.ts`
- `src/services/email/providerFactory.ts`

Later:

- large parts of `src/services/gmail/syncManager.ts`

## Risks

- IMAP message ID handling is currently encoded in TS and must be preserved carefully when moved.
- Some frontend code still assumes access to decrypted account data. That cannot be fully removed until IMAP config building and ad-hoc IMAP operations move to Rust.
- Sync and post-sync logic are user-visible and should not be migrated in the same patch set as auth/account changes.
- IMAP OAuth refresh behavior must be made consistent across all Rust-side operations before the TS IMAP provider can be collapsed.

## Open Questions

- Should Rust fully own browser-opening for OAuth, or should it return an auth URL and let the frontend open it?
- Should desktop notifications remain frontend-side permanently, with Rust only emitting notification candidates?
- Do we want the frontend to keep a small invoke-backed provider adapter, or should provider operations move directly into `src/core/` facade functions?
- Should IMAP/SMTP auto-discovery eventually move to Rust as part of the native-frontend effort, or remain a TS-side compatibility layer for now?

## Recommended Execution Order

1. Account/auth backend ownership
2. Unified provider test/profile/folder CRUD
3. IMAP semantics migration
4. Gmail/JMAP/Graph normalization cleanup
5. Sync orchestration migration
6. Post-sync automation migration
7. Final removal of legacy TS provider glue

## Progress Log

- 2026-03-11: Initial migration plan written based on current repo structure and command surface.
