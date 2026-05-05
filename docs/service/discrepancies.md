# Phase 5 review discrepancies

Consolidated findings from the post-landing review sweep (arch + bugs, claude + codex; codex arch failed and is not represented). Each finding is tagged with the reviewer(s) that raised it - duplicate findings across reviewers are signal, not noise. Editorial priority is mine.

**Workflow.** When a finding is fixed, **remove it from this file**. When the file is empty, **delete it**. This document is a working punch list, not a record - the git history is the record.

---

## Blockers

These are correctness or architectural violations that breach Phase 5's premise.

**JMAP email sync still writes calendar rows directly, bypassing `CalendarRuntime`.** [discovered during fix work]
- `crates/jmap/src/sync/mod.rs:222` (initial) and `:415` (delta) call `jmap::calendar_sync::sync_calendars` from inside the email sync path.
- The Gmail/Graph variants of this finding were fixed; JMAP wasn't in the codex scope and the calendar crate has no `sync_jmap_calendar_account` path today (`calendar/src/sync.rs` routes only `google_api`/`graph`/`caldav`).
- Removing the JMAP bypass without adding a CalendarRuntime path would silently break calendar sync for Fastmail accounts. Two-step fix: (a) add a JMAP arm to `calendar_sync_account_impl`'s match (likely needs the calendar crate to depend on jmap and a `JmapState`-like factory); (b) delete the bypass calls. Both in one commit.

---

## Should fix

Real gaps with measurable impact, ordered roughly by how often the user-facing behavior shows up.

**`mutated` flag is coarser than the plan stipulates - partial-failure runs don't notify the UI.** [claude arch, claude bugs, codex bugs]
- `crates/service/src/calendar.rs:399-403` sets `mutated = false` for any non-cancelled `Failed` outcome.
- But the discovered-calendars upsert at `crates/calendar/src/sync.rs:124-141` (and the analogous upserts for Google/Graph/CalDAV per the codex pointers) commits *before* per-calendar event loops run. A failure mid-loop after that upsert leaves the UI stale until the next successful run.
- The plan's § "CalendarChanged: partial-mutation emission" explicitly forbids this exact behavior: "A run cancelled or failed *after* a committed batch has already mutated calendar rows; the UI must reload to surface them."
- Module doc-comment at `service/src/calendar.rs:48-66` and `problem-statement.md` § "Phase 5 known gaps" both acknowledge this as deferred to Phase 6 alongside the `cal::sync` helper refactor that exposes per-batch mutation tracking. So this is a known deviation, not an undiscovered defect - but three reviewers caught it independently, and the deviation is not load-bearing in the plan: it's a "we ran out of refactor budget" deferral, not a "we changed our mind" decision.
- Either close the gap in Phase 5 (helper refactor) or move the deferral note to a more prominent location so the next reviewer doesn't keep re-flagging it.

**Plan's Phase-5-scoped unit test cohort largely did not land.** [claude arch, claude bugs]
- Plan task 13 listed eight in-scope tests; three landed (wire-type round-trips, `CalendarRuntime` shutdown-guard, catalog cross-respawn). Five did not:
  - GAL handler mutex test (the plan calls this out as **required** in `service/src/handlers/gal.rs:10-30` for correctness verification of the `NOTIFY_CAP=4` duplicate-call hazard).
  - Notification-drain timeout test - novel `drain_notifications_bounded` with the `spawn_blocking` caveat is unverified.
  - IMAP cancellation unit tests (mid-folder / per-chunk).
  - Calendar cancellation unit tests against `calendar_sync_account_impl` with stub providers.
  - `cancel_and_await_cancels_calendar` integration test.
  - Mirror unit tests for `pending_calendars` map (sync side has three at `service_client.rs:2992-3072`; calendar has none).
- None of these depend on the fake-CalDAV fixture that justified the Phase 8 deferral - they are shape/lifecycle tests.
- The "Phase 5 status (as landed)" doc only acknowledges integration cohort deferral; it does not enumerate which sub-tests slipped from "in scope" to "deferred." Either land the tests or update the doc to be honest about the slip.

**Explicit calendar request path is exposed but not wired.** [codex bugs]
- `start_calendar_sync` exists at `crates/app/src/service_client.rs:924`. Grep finds no call sites.
- Plan task 9b explicitly listed call-site updates: post-account-creation in `handlers/account_*.rs`, manual "sync now", RSVP-then-resync. None of these landed.
- Account-added path (`crates/app/src/handlers/accounts.rs:11`) ignores the new account ID and only reloads accounts.
- Manual sync (`crates/app/src/update.rs:366`) still calls email sync only.
- This also explains why the `pending_calendars` leak above never gets swept in practice.

**`CalendarReloadTick` subscription wakes every 250 ms unconditionally.** [claude bugs]
- `crates/app/src/subscription.rs:143-146` is not gated on calendar visibility or even on having any calendar-capable accounts. Sibling `ReaderReloadTick` (`:131`) is gated on `self.search_state.is_some()`.
- Handler returns immediately when `pending_calendar_reload` is `None`, so cost is small per tick - but ~4 wakeups/sec, 24/7, for a feature most users don't have open.
- Match the gating pattern: `self.calendar.is_active()` or `self.sidebar.accounts.iter().any(|a| has_calendar_provider(a))`.

**Calendar cancellation depth doesn't reach the per-event-batch boundary the plan promised.** [claude bugs]
- Plan task 3a called for point-checks at "calendar-list-entry, per-calendar-entry, per-event-batch boundaries."
- Calendar-list and per-calendar landed (`crates/core/src/caldav/sync.rs:43,62`; `crates/calendar/src/sync.rs:289,318`).
- Per-event-batch did not. `sync_calendar_events` doesn't take a `&CancellationToken`; the per-resource upsert loop at `crates/core/src/caldav/sync.rs:235` cannot observe cancellation. Google/Graph paths have the same gap inside `*_sync_events_impl`.
- Real-world impact small (these are local DB writes, not network), but a calendar with thousands of events spends seconds in this loop oblivious to cancellation. Plan promised it; code didn't deliver.

**`imap_delta_janitor` flag-sync helpers don't accept the cancellation token.** [claude arch, codex bugs]
- `sync_flags_on_session` / `sync_flags_without_condstore` (`crates/imap/src/imap_delta_janitor.rs:32`,`:109`) have no `CancellationToken` parameter. Multiple RPCs (`fetch_all_flags`, `with_conn` diff, `with_conn` apply) run unchecked.
- Also: `run_deletion_detection` swallows cancellation into an empty result (`:228`); the caller at `crates/imap/src/imap_delta.rs:250` returns `Ok` with zero messages, which Service maps to `Completed` rather than `Cancelled`.
- The plan's scope expansion explicitly named `imap_delta_janitor`. Cancel latency in the non-CONDSTORE path is `IMAP_FETCH_TIMEOUT` rather than per-RPC - the plan accepts that bound, but only as an upper bound. The helper-level gap means the bound holds by accident, not by construction.

**Kick handler iterates *all* accounts, not calendar-capable ones.** [claude arch]
- `handle_calendar_kick` calls `list_all_account_ids` and feeds the unfiltered list into `accounts_due_for_sync` -> `start_account`.
- For an IMAP-only or JMAP-only account, the runner reaches `calendar_sync_account_impl`, fails through to `Err("No calendar provider configured for account ...")`, and stamps `last_completed`. Each hour, the same accounts re-fail with the same log line.
- Plan called out the analogous widened-load for GAL but not the calendar variant. Nuisance log noise on multi-account installs that mix providers.
- Filter at enumeration: `calendar_provider IS NOT NULL OR provider IN (...)`.

---

## Out-of-Phase-5-scope but flagged here so it doesn't get lost

**Gmail and Graph cancellation is shallow at the entry point only.** [codex bugs]
- Both providers carry a `cancellation_token` field marked dead (`crates/gmail/src/sync/mod.rs:35`, `crates/graph/src/sync/mod.rs:50`) and only check it at entry. Long loops continue and can return `Ok`, which Service reports as `Completed` rather than `Cancelled` (`crates/service/src/sync.rs:409`).
- Phase 3 was supposed to bring Gmail/Graph to checkpoint depth. The Phase 5 plan asserts it did. Codex's read says it didn't - or at least that the depth is inadequate.
- Not strictly a Phase 5 finding, but worth a Phase 3 retrospective check before next phase.

---

## Nits

**Two duplicate `list_all_account_ids` helpers.** [claude arch]
- Identical bodies in `crates/service/src/handlers/calendar.rs:108-126` and `crates/service/src/handlers/gal.rs:100-114`. The calendar handler's docstring acknowledges and defers; now that two callers exist, lift to `BootSharedState` or `db::queries_extra`.

**`encryption_key_bytes` allocates and copies on every run.** [claude arch]
- `crates/service/src/calendar.rs:374-377`. `SecretKey::expose()` returns `&[u8; 32]`; one-liner is `let encryption_key_bytes = *inner.encryption_key.expose();`.

**`GmailState` / `GraphState` constructed on every run.** [claude arch]
- `service/src/calendar.rs`. They're cheap, but storing once on `CalendarRuntimeInner` reduces per-run noise.

**Doc-comment line-number drift in GAL handler.** [claude arch]
- `service/src/handlers/gal.rs:35` cites `crates/core/src/contacts/gal.rs:212` as the `spawn_blocking` site. Current file has `cache_gal_entries(...).await?` at that line; the actual `spawn_blocking` is inside `ReadDbState::with_conn`. Same pattern as the plan's earlier "line-number drift updated" pass.

**No dedicated serde round-trip tests for `Calendar*Params` types.** [claude bugs]
- `CalendarStartAccountSyncParams` / `CalendarCancelAccountSyncParams` (`service-api/src/calendar.rs:59,80`). The `RequestParams::from_method_params` round-trip exercises them transitively, but a dedicated test would catch a silent field rename.

**`service_generation` is captured at `CalendarRuntime` construction (= 0) and never updated.** [claude bugs]
- `crates/service/src/dispatch.rs:952-957`. The reader task overwrites with `current_generation` at enqueue per the `WithGeneration` contract. Same pattern as `SyncRuntime` at `boot.rs:803`. Documented and consistent; flagging only because the field looks load-bearing on the Service side and a future contributor might wonder. Add a one-line comment at the construction site.

**`unwrap_or(true)` in `start_account`'s opportunistic-cleanup pass.** [claude bugs]
- `service/src/calendar.rs:178-191`. `supervisor: None` only happens mid-shutdown, which the `closed` re-check after lock-acquire already rejects. The `unwrap_or(true)` looks like it's protecting against a state that's already prevented. Clarity nit.
