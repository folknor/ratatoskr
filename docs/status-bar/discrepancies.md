# Status Bar: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### Sync progress subscription not connected to sync orchestrator
`IcedProgressReporter` and `create_sync_progress_channel()` are implemented in `status_bar.rs:143-166`. The sync orchestrator does not use them. No code in `main.rs` creates the channel, polls the receiver, or passes the reporter to sync. `Message::SyncProgress(SyncEvent)` variant exists but nothing sends it. All five status bar inbound data methods (`report_sync_progress`, `report_sync_complete`, `set_warning`, `clear_warning`, `show_confirmation`) are reachable only through `handle_sync_event` which has no upstream data source.
- Code: `crates/app/src/ui/status_bar.rs:160-166` (channel factory, unused)
- Code: `crates/app/src/main.rs:733-736` (dispatch exists, no sender)

### Confirmation dispatch points not wired
`show_confirmation()` is called only from the placeholder reauth handler (`main.rs:1051`). No email action handlers call it (archive, trash, label, star, etc.) because `Message::EmailAction` is a no-op stub (`main.rs:619`).
- Spec: `docs/status-bar/implementation-spec.md` section 11.1 (dispatch table)
- Code: `crates/app/src/main.rs:619` (`EmailAction(_action) => Task::none()`)

### Token expiry warnings not wired to auth errors
The auth error handling path does not exist. `set_warning()` with `WarningKind::TokenExpiry` has no call site. Only `ConnectionFailure` warnings can flow via `SyncEvent::Error`, which itself has no upstream source (see above).
- Spec: `docs/status-bar/implementation-spec.md` section 10.1
- Code: `crates/app/src/ui/status_bar.rs:271-273` (method exists, uncalled)

### RequestReauth shows placeholder confirmation instead of re-auth flow
`handle_status_bar_event` logs to stderr and shows "not yet implemented" confirmation instead of opening a re-authentication flow. Depends on accounts Phase 7 `ReauthWizard` which does not exist.
- Code: `crates/app/src/main.rs:1046-1056`

### `prune_stale_sync` and `begin_sync_generation` never called
The generational tracking methods exist but no code calls them. `begin_sync_generation` and `prune_stale_sync` are dead code until the sync orchestrator is connected.
- Code: `crates/app/src/ui/status_bar.rs:237-264` (methods exist, uncalled)

### `warnings` uses BTreeMap instead of spec's HashMap
Spec defines `HashMap<String, AccountWarning>`. Code uses `BTreeMap<String, AccountWarning>`. This provides deterministic cycling order — an improvement, not a bug.
- Spec: `docs/status-bar/implementation-spec.md` section 2.1
- Code: `crates/app/src/ui/status_bar.rs:187`
