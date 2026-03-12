# Frontend to Backend Migration Plan

## Goal

Continue moving business logic out of the TypeScript/Tauri frontend and into the Rust backend while keeping the current frontend functional.

The long-term target is a native frontend. Rust should remain the canonical owner of:

- account creation and lifecycle
- auth and token refresh
- secret encryption/decryption
- provider operations
- sync orchestration
- provider-specific normalization and semantics

The frontend should keep shrinking toward:

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

Most of the original migration plan is now complete.

Rust already owns most of:

- account creation and reauthorization flows
- token refresh and secret decryption on read
- provider test/profile/fetch/folder CRUD
- IMAP config building and OAuth freshness for provider operations
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
- desktop notification display
- a few remaining settings/account compatibility reads
- some legacy compatibility wrappers and tests

## Boundary Status

### Largely complete

- account/auth ownership
- unified provider API
- IMAP semantics migration
- provider normalization cleanup
- sync orchestration migration
- most settings/account bootstrap snapshot work
- folder/label cleanup

### Still intentionally incomplete

- final AI execution boundary
- final post-sync UI/backend boundary
- compatibility sweeps for remaining full account/settings reads
- broader regression coverage around migrated sync/bootstrap behavior

## Remaining Work

### 1. Decide the final AI execution boundary

Current state:

- provider/runtime/config selection is already Rust-backed
- TypeScript still assembles prompts and invokes AI for:
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

- keep inference calls in TypeScript, with Rust remaining the owner of provider/runtime/config
- or move prompt execution into Rust via typed AI task commands

Recommendation:

- do not move this blindly
- first decide whether the native-frontend target wants Rust to own:
  - prompt templates
  - output parsing/validation
  - AI task-specific DTOs

If yes, introduce task-specific commands such as:

- `ai_summarize_thread`
- `ai_generate_smart_replies`
- `ai_transform_text`
- `ai_extract_task`
- `ai_generate_auto_draft`

### 2. Finish the post-sync boundary intentionally

Current TypeScript-owned remainder:

- actual desktop notification display
- actual AI smart-label inference call
- actual AI categorization inference call

Current hot spot:

- `src/services/gmail/syncManager.ts`

Concrete next step:

- decide whether `syncManager.ts` should remain a thin event subscriber permanently
- if so, keep only:
  - notification display
  - UI progress shaping
- move any remaining policy decisions still embedded in TypeScript

### 3. Sweep remaining settings/account compatibility reads

Most of the high-value settings/account work is already done:

- settings bootstrap snapshot exists
- UI bootstrap snapshot exists
- account summary DTOs cover most app/window use
- CalDAV settings now use a narrow Rust-backed DTO

The remaining work here is a sweep, not a major migration phase.

Current likely targets:

- `src/services/db/accounts.ts`
- `src/services/db/settings.ts`
- parts of `src/services/gmail/tokenManager.ts`
- any remaining one-off `getAccount()` or `getSetting()` reads outside the new snapshot paths

Guideline:

- prefer targeted cleanup over broad TypeScript beautification
- replace full-row/settings reads with narrow Rust DTOs when they show up in active paths

### 4. Strengthen regression coverage around migrated sync behavior

Current gap:

- architecture moved faster than broad regression coverage
- `syncManager` regression coverage improved, but broader migrated flows are still thinner than ideal

Concrete next step:

- add focused tests for:
  - sync status event handling
  - background sync start/stop behavior
  - post-sync hook triggering
  - account bootstrap paths that now use account-summary DTOs

## Likely Remaining Files to Shrink

- `src/services/gmail/syncManager.ts`
- `src/services/db/accounts.ts`
- `src/services/db/settings.ts`
- `src/services/gmail/tokenManager.ts`
- AI task-specific services that still assemble prompts client-side

## Current Risks

- the remaining TypeScript AI call layer is now the largest intentionally un-migrated business-logic seam
- sync behavior is user-visible, and regression coverage should keep catching up
- TypeScript compatibility wrappers can still hide stale call paths if they are not periodically audited

## Recommended Execution Order

1. Decide the final boundary for AI task execution
2. Finish the post-sync boundary intentionally, keeping only the UI-owned pieces in TypeScript
3. Continue small sweeps for remaining full-account/settings compatibility reads
4. Expand regression coverage around migrated sync/bootstrap paths
5. Remove leftover compatibility glue opportunistically as those areas are touched

## Progress Log

- 2026-03-11: Initial migration plan written based on repo structure and command surface at the time.
- 2026-03-11: Major migration progress completed across account/auth, provider unification, IMAP semantics, sync orchestration, post-sync hooks, calendar providers, and AI runtime/config ownership.
- 2026-03-11: Plan updated to reflect current status and concrete remaining work instead of the original broad migration phases.
- 2026-03-11: Settings/account snapshot work and syncManager regression coverage updated; remaining work narrowed to AI boundary decisions, post-sync finishing, compatibility sweeps, and broader regression coverage.
- 2026-03-12: Plan simplified again to remove stale pre-migration phase detail and reflect the current active backlog after the Phase 1/1.5/2 decoupling and follow-up cleanup work landed.
