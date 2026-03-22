# Status Bar: Spec vs. Code Discrepancies

Audit date: 2026-03-22

---

## Resolved (previously open)

- Sync progress subscription connected to sync orchestrator (channel, receiver, subscription, dispatch all wired)
- Confirmation dispatch points wired (handle_email_action + reauth + account save + compose send)
- RequestReauth handler calls handle_open_reauth_wizard (no longer a placeholder)

## Remaining

### Token expiry warnings not wired to auth errors
`WarningKind::TokenExpiry` type, UI, click-to-reauth handler all exist. No code path calls `set_warning()` with `TokenExpiry` when OAuth refresh fails or tokens expire. Tracked in TODO.md.

### `begin_sync_generation` and `prune_stale_sync` never called
Generational tracking methods exist but sync orchestrator does not call them. Tracked in TODO.md under "Connect sync orchestrator to IcedProgressReporter".

## Not a discrepancy

### `warnings` uses BTreeMap instead of HashMap
Intentional improvement for deterministic cycling order.
