# Status Bar: Problem Statement

## Overview

A thin horizontal bar at the bottom of the main window. Always visible, never obtrusive. It communicates ongoing activity (sync progress), persistent warnings (auth failures, connection errors), and transient confirmations (action completed).

The status bar exists in both mail and calendar modes. It does not appear in pop-out windows (message detail, calendar pop-out).

## Layout

A single row, full window width, minimal height (one line of text plus padding).

```
┌──────────────────────────────────────────────────────────────────┐
│ Main app UI                                                      │
│                                                                  │
│                                                                  │
├──────────────────────────────────────────────────────────────────┤
│ ⟳ Syncing alice@corp.com (1,247 / 8,302)                        │
└──────────────────────────────────────────────────────────────────┘
```

**Left side:** Status message (sync progress, warnings, confirmations).
**Right side:** Reserved for future use (e.g., connection indicator, notification count).

## Content Types

### Sync Progress

Shown when one or more accounts are actively syncing.

**Single account syncing:**
```
⟳ Syncing alice@corp.com (1,247 / 8,302)
```

**Multiple accounts syncing:**
```
⟳ Syncing 3 accounts... (alice@corp.com: 1,247 / 8,302)
```

When multiple accounts sync simultaneously, the status bar shows a summary with the account count, and cycles through individual account progress on a short interval (~3 seconds). The cycling is automatic - the user does not need to interact. The currently displayed account's progress is shown in parentheses.

**Sync complete:** The progress message disappears. No "sync complete" confirmation - the absence of the spinner is the signal.

### Warnings

Persistent messages that remain until the underlying issue is resolved. Warnings take priority over sync progress - if both a warning and sync progress are active, the warning is shown.

**Token expiry:**
```
⚠ alice@corp.com needs re-authentication - click to sign in
```

Clicking the warning opens the re-authentication flow (OAuth or password, depending on the account).

**Connection failure:**
```
⚠ alice@corp.com - connection failed (timeout)
```

**Multiple warnings:** If multiple accounts have issues, cycle through them on the same interval as sync progress, with a count prefix:

```
⚠ 2 accounts need attention - alice@corp.com needs re-authentication
```

### Transient Confirmations

Brief messages (~3 seconds) confirming completed actions. Lower priority than warnings and sync progress - they are shown only when nothing else is competing for the status bar.

Examples:
- "Message moved to Trash"
- "Label applied"
- "Event created"

These fade out automatically after the display duration.

## Priority Order

When multiple content types compete for the status bar:

1. **Warnings** - always win, they indicate something is broken. Never preempted.
2. **Sync progress** - the steady-state default when no warnings are active. May be briefly preempted by confirmations (see below).
3. **Transient confirmations** - lowest priority, shown when nothing else is active.

**Confirmation preemption rule:** If a transient confirmation arrives while sync progress is showing, the confirmation briefly interrupts the progress (~3 seconds), then progress resumes. This is an exception to the strict priority order - confirmations are lower priority than sync, but a brief interruption is acceptable because the user just performed an action and deserves immediate feedback. Warnings are never preempted by anything.

## Interaction

The status bar is not interactive except for warning messages, which are clickable to initiate the relevant recovery action (re-authentication, retry connection, etc.). The cursor changes to indicate clickability on warnings.

## Visual Style

Minimal - same background as the app chrome, slightly smaller text than the main UI. Warning messages use a warning color for the icon/text. Sync progress uses a muted/secondary text color. The bar should be visually quiet when everything is healthy.

## Ecosystem Patterns

How requirements from this spec map to patterns found in the [iced ecosystem survey](../iced-ecosystem-survey.md).

### Requirements → Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Cycling timer + concurrent sync | pikeru subscriptions + rustcast `Subscription::batch()` | Multiplex timer ticks, sync events, and confirmation expiry in one subscription |
| Encapsulated panel | trebuchet Component trait | StatusBar component with own state, view, subscription; emits `RequestReauth(account_id)` upward |
| Visual styling | shadcn-rs + iced-plus tokens | Tokens for muted text, warning color, chrome background |
| Clickable warnings + cursor change | bloom custom Widget + pikeru MouseArea | Custom `Widget` with conditional `mouse_interaction()` returning Pointer for warnings |
| Priority content switching | rustcast Page enum | `StatusContent` enum (Idle/SyncProgress/Warning/Confirmation) with automatic priority resolution |
| Stale sync state | bloom generational tracking | Per-account generation map (extended from bloom's single counter) |

### Gaps

- **Priority-based preemption logic** (warnings interrupt sync, confirmations briefly interrupt then yield back): Bespoke state machine, no survey precedent
