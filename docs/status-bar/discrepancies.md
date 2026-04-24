# Status Bar: Spec vs. Code Discrepancies

Audit date: 2026-03-22

---

## Resolved

- Sync progress subscription connected to sync orchestrator (channel, receiver, subscription, dispatch all wired)
- Confirmation dispatch points wired (handle_email_action + reauth + account save + compose send)
- RequestReauth handler calls handle_open_reauth_wizard (no longer a placeholder)
- Token expiry warnings wired: SyncComplete handler detects auth errors and sets WarningKind::TokenExpiry. Connection failures set ConnectionFailure. Successful sync clears warnings.
- Sync orchestrator connected: initial sync on boot, periodic delta sync every 5 min, IcedProgressReporter passed via ProviderCtx

## Remaining

### `begin_sync_generation` and `prune_stale_sync` never called
Generational tracking methods exist on StatusBar but the sync dispatch does not call them. Low priority - sync progress display works without them. These are for cleaning up stale progress entries when a sync cycle is interrupted.

## Not a discrepancy

### `warnings` uses BTreeMap instead of HashMap
Intentional improvement for deterministic cycling order.
