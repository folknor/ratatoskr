# Status Bar: Implementation Spec

## Overview

A single-row bar at the bottom of the main window that displays sync progress, persistent warnings, and transient confirmations. Implemented as a `Component` (same pattern as `Sidebar`, `ThreadList`, `ReadingPane`) with its own state machine, subscription, and view. The status bar is small but touches sync, auth, and command execution — this spec is precise about the integration boundaries.

References: [problem statement](./problem-statement.md), [accounts spec](../accounts/problem-statement.md).

---

## 1. Component Structure

### File: `crates/app/src/ui/status_bar.rs`

The status bar is a `Component` implementing `update`, `view`, and `subscription`. It does not own any database handles — all data flows in via messages from the parent `App`.

### Module Registration

Add `pub mod status_bar;` to `crates/app/src/ui/mod.rs`. Add `status_bar: StatusBar` to the `App` struct in `main.rs`.

---

## 2. Types

### 2.1 Core State

```rust
pub struct StatusBar {
    /// Currently active warnings, keyed by account ID.
    warnings: Vec<AccountWarning>,

    /// Per-account sync progress, keyed by account ID.
    sync_progress: HashMap<String, SyncAccountProgress>,

    /// Currently displayed transient confirmation, if any.
    confirmation: Option<Confirmation>,

    /// Index for cycling through multiple warnings or syncing accounts.
    cycle_index: usize,
}
```

### 2.2 Warning State

```rust
#[derive(Debug, Clone)]
pub struct AccountWarning {
    pub account_id: String,
    pub email: String,
    pub kind: WarningKind,
}

#[derive(Debug, Clone)]
pub enum WarningKind {
    /// OAuth token expired or refresh failed. Clickable — opens re-auth.
    TokenExpiry,
    /// Persistent connection failure. Not clickable (no recovery action).
    ConnectionFailure { message: String },
}
```

### 2.3 Sync Progress State

```rust
#[derive(Debug, Clone)]
pub struct SyncAccountProgress {
    pub email: String,
    pub current: u64,
    pub total: u64,
    pub phase: String,
}
```

### 2.4 Confirmation State

```rust
#[derive(Debug, Clone)]
struct Confirmation {
    text: String,
    /// When the confirmation was created. Used to compute expiry.
    created_at: iced::time::Instant,
}
```

### 2.5 Resolved Display Content

The view function resolves the current display state from the priority rules. This is not stored — it is computed fresh on every `view()` call.

```rust
enum ResolvedContent<'a> {
    /// Nothing to show. The bar is empty (healthy idle state).
    Idle,
    /// One or more warnings. Fields carry the formatted display string
    /// and whether the warning is clickable.
    Warning {
        text: String,
        clickable: bool,
        /// The account_id for the currently displayed warning (for click handler).
        account_id: String,
    },
    /// Sync in progress. Formatted display string.
    SyncProgress { text: String },
    /// Transient confirmation message.
    Confirmation { text: String },
}
```

---

## 3. Message and Event Enums

### 3.1 `StatusBarMessage` (internal)

```rust
#[derive(Debug, Clone)]
pub enum StatusBarMessage {
    /// Timer tick for cycling through multiple warnings/accounts and
    /// expiring confirmations. Fires every ~3 seconds.
    CycleTick(iced::time::Instant),

    /// User clicked a clickable warning.
    WarningClicked,
}
```

### 3.2 `StatusBarEvent` (outward to App)

```rust
#[derive(Debug, Clone)]
pub enum StatusBarEvent {
    /// User clicked a token expiry warning. The app should initiate
    /// re-authentication for this account.
    RequestReauth { account_id: String },
}
```

### 3.3 Inbound Data Methods (called by App, not via Message)

The `App` pushes data into the status bar by calling methods directly on `StatusBar`, not by routing through `update()`. This avoids unnecessary message indirection for data that flows one-way from the app.

```rust
impl StatusBar {
    /// Called by App when sync progress arrives from the ProgressReporter.
    pub fn report_sync_progress(
        &mut self,
        account_id: String,
        email: String,
        current: u64,
        total: u64,
        phase: String,
    );

    /// Called by App when an account finishes syncing (removes from map).
    pub fn report_sync_complete(&mut self, account_id: &str);

    /// Called by App when an account warning is detected (token expiry,
    /// connection failure). Replaces any existing warning for this account.
    pub fn set_warning(&mut self, warning: AccountWarning);

    /// Called by App when a warning condition is resolved (e.g., successful
    /// re-auth, connection restored).
    pub fn clear_warning(&mut self, account_id: &str);

    /// Called by App after a user action completes (move, label, etc.).
    pub fn show_confirmation(&mut self, text: String);
}
```

---

## 4. State Machine: Priority Resolution

Priority is resolved in `view()`, not stored. The `resolve()` method computes what to display:

```rust
impl StatusBar {
    fn resolve(&self, now: iced::time::Instant) -> ResolvedContent<'_> {
        // 1. Warnings always win (never preempted).
        if !self.warnings.is_empty() {
            return self.resolve_warning();
        }

        // 2. Active confirmation briefly preempts sync progress.
        //    (Confirmation preemption exception from the problem statement.)
        if let Some(ref conf) = self.confirmation {
            if now.duration_since(conf.created_at) < CONFIRMATION_DURATION {
                return ResolvedContent::Confirmation {
                    text: conf.text.clone(),
                };
            }
        }

        // 3. Sync progress is the steady-state default.
        if !self.sync_progress.is_empty() {
            return self.resolve_sync_progress();
        }

        // 4. Confirmation that arrived with no sync active.
        if let Some(ref conf) = self.confirmation {
            if now.duration_since(conf.created_at) < CONFIRMATION_DURATION {
                return ResolvedContent::Confirmation {
                    text: conf.text.clone(),
                };
            }
        }

        // 5. Nothing to show.
        ResolvedContent::Idle
    }
}
```

### 4.1 Constants

```rust
/// How long transient confirmations are visible.
const CONFIRMATION_DURATION: std::time::Duration = std::time::Duration::from_secs(3);

/// Cycle interval for rotating through multiple warnings or syncing accounts.
const CYCLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
```

### 4.2 Warning Resolution

```rust
fn resolve_warning(&self) -> ResolvedContent<'_> {
    let idx = self.cycle_index % self.warnings.len();
    let warning = &self.warnings[idx];
    let clickable = matches!(warning.kind, WarningKind::TokenExpiry);

    let text = if self.warnings.len() == 1 {
        match &warning.kind {
            WarningKind::TokenExpiry => {
                format!("{} needs re-authentication \u{2014} click to sign in", warning.email)
            }
            WarningKind::ConnectionFailure { message } => {
                format!("{} \u{2014} connection failed ({})", warning.email, message)
            }
        }
    } else {
        let detail = match &warning.kind {
            WarningKind::TokenExpiry => {
                format!("{} needs re-authentication", warning.email)
            }
            WarningKind::ConnectionFailure { message } => {
                format!("{} \u{2014} {}", warning.email, message)
            }
        };
        format!("{} accounts need attention \u{2014} {}", self.warnings.len(), detail)
    };

    ResolvedContent::Warning {
        text,
        clickable,
        account_id: warning.account_id.clone(),
    }
}
```

### 4.3 Sync Progress Resolution

```rust
fn resolve_sync_progress(&self) -> ResolvedContent<'_> {
    let accounts: Vec<&SyncAccountProgress> = self.sync_progress.values().collect();

    let text = if accounts.len() == 1 {
        let p = accounts[0];
        format!("Syncing {} ({} / {})", p.email, format_number(p.current), format_number(p.total))
    } else {
        let idx = self.cycle_index % accounts.len();
        let p = accounts[idx];
        format!(
            "Syncing {} accounts... ({}: {} / {})",
            accounts.len(),
            p.email,
            format_number(p.current),
            format_number(p.total),
        )
    };

    ResolvedContent::SyncProgress { text }
}
```

### 4.4 Number Formatting Helper

```rust
/// Format a number with thousands separators (e.g., 1247 -> "1,247").
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}
```

---

## 5. Component Implementation

### 5.1 `update()`

```rust
impl Component for StatusBar {
    type Message = StatusBarMessage;
    type Event = StatusBarEvent;

    fn update(
        &mut self,
        message: StatusBarMessage,
    ) -> (Task<StatusBarMessage>, Option<StatusBarEvent>) {
        match message {
            StatusBarMessage::CycleTick(now) => {
                self.cycle_index = self.cycle_index.wrapping_add(1);

                // Expire confirmation if past its duration.
                if let Some(ref conf) = self.confirmation {
                    if now.duration_since(conf.created_at) >= CONFIRMATION_DURATION {
                        self.confirmation = None;
                    }
                }

                (Task::none(), None)
            }
            StatusBarMessage::WarningClicked => {
                if self.warnings.is_empty() {
                    return (Task::none(), None);
                }
                let idx = self.cycle_index % self.warnings.len();
                let warning = &self.warnings[idx];
                match warning.kind {
                    WarningKind::TokenExpiry => {
                        let event = StatusBarEvent::RequestReauth {
                            account_id: warning.account_id.clone(),
                        };
                        (Task::none(), Some(event))
                    }
                    WarningKind::ConnectionFailure { .. } => {
                        // Not clickable — no event.
                        (Task::none(), None)
                    }
                }
            }
        }
    }
}
```

### 5.2 `subscription()`

The status bar needs a periodic tick when it has content that cycles (multiple warnings, multiple syncing accounts) or content that expires (confirmation). When idle, no subscription is needed.

```rust
fn subscription(&self) -> iced::Subscription<StatusBarMessage> {
    let needs_tick = self.warnings.len() > 1
        || self.sync_progress.len() > 1
        || self.confirmation.is_some();

    if needs_tick {
        iced::time::every(CYCLE_INTERVAL).map(StatusBarMessage::CycleTick)
    } else {
        iced::Subscription::none()
    }
}
```

**Note on confirmation expiry timing:** When a single confirmation is the only content (no cycling needed), the tick subscription still runs so the confirmation can be expired after `CONFIRMATION_DURATION`. The tick interval equals the confirmation duration (both 3s), so the confirmation expires on the first tick after creation. This is acceptable — sub-second precision is not needed for a "roughly 3 seconds" display.

### 5.3 `view()`

```rust
fn view(&self) -> Element<'_, StatusBarMessage> {
    let now = iced::time::Instant::now();
    let content = self.resolve(now);

    match content {
        ResolvedContent::Idle => {
            // Empty bar — still render the container for consistent height.
            container(Space::new().height(0))
                .width(Length::Fill)
                .height(STATUS_BAR_HEIGHT)
                .style(ContainerClass::StatusBar.style())
                .into()
        }
        ResolvedContent::Warning { text, clickable, .. } => {
            let icon_el = icon::alert_triangle()
                .size(ICON_MD)
                .style(TextClass::Warning.style());
            let text_el = iced::widget::text(text)
                .size(TEXT_SM)
                .style(TextClass::Warning.style());

            let row = row![
                container(icon_el).align_y(Alignment::Center),
                container(text_el).align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);

            let bar = container(row)
                .padding(PAD_STATUS_BAR)
                .width(Length::Fill)
                .height(STATUS_BAR_HEIGHT)
                .style(ContainerClass::StatusBar.style());

            if clickable {
                mouse_area(bar)
                    .on_press(StatusBarMessage::WarningClicked)
                    .interaction(iced::mouse::Interaction::Pointer)
                    .into()
            } else {
                bar.into()
            }
        }
        ResolvedContent::SyncProgress { text } => {
            let icon_el = icon::refresh()
                .size(ICON_MD)
                .style(TextClass::Muted.style());
            let text_el = iced::widget::text(text)
                .size(TEXT_SM)
                .style(TextClass::Muted.style());

            let row = row![
                container(icon_el).align_y(Alignment::Center),
                container(text_el).align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);

            container(row)
                .padding(PAD_STATUS_BAR)
                .width(Length::Fill)
                .height(STATUS_BAR_HEIGHT)
                .style(ContainerClass::StatusBar.style())
                .into()
        }
        ResolvedContent::Confirmation { text } => {
            let icon_el = icon::check()
                .size(ICON_MD)
                .style(TextClass::Muted.style());
            let text_el = iced::widget::text(text)
                .size(TEXT_SM)
                .style(TextClass::Muted.style());

            let row = row![
                container(icon_el).align_y(Alignment::Center),
                container(text_el).align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);

            container(row)
                .padding(PAD_STATUS_BAR)
                .width(Length::Fill)
                .height(STATUS_BAR_HEIGHT)
                .style(ContainerClass::StatusBar.style())
                .into()
        }
    }
}
```

---

## 6. Layout Constants

Add to `crates/app/src/ui/layout.rs`:

```rust
/// Status bar fixed height (one line of text + padding).
pub const STATUS_BAR_HEIGHT: f32 = 28.0;

/// Status bar internal padding (compact vertical, standard horizontal).
pub const PAD_STATUS_BAR: Padding = Padding {
    top: 4.0,
    right: 12.0,
    bottom: 4.0,
    left: 12.0,
};
```

The height is derived from: `TEXT_SM` (11px) line height (~15px) + vertical padding (4 + 4 = 8px) + small margin. Rounding to 28px keeps the bar compact. All values land on the spacing scale (`SPACE_XXS` = 4, `SPACE_SM` = 12).

---

## 7. Theme Additions

### 7.1 Container Style

Add to `ContainerClass` in `crates/app/src/ui/theme.rs`:

```rust
/// Status bar background.
StatusBar,
```

Implementation:

```rust
fn style_status_bar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weaker.color.into()),
        border: iced::Border {
            color: theme.palette().background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}
```

The status bar uses `background.weaker` — the same level as the sidebar. The top border (1px, 10% alpha of `strongest`) provides a subtle separator from the main content area.

### 7.2 Warning Text Style

Add to `TextClass` in `crates/app/src/ui/theme.rs`:

```rust
/// Warning text/icon color (for status bar warnings).
Warning,
```

Implementation:

```rust
fn style_text_warning(theme: &Theme) -> text::Style {
    text::Style { color: Some(theme.palette().warning.base.color) }
}
```

This uses the theme's `warning` seed color, so it adapts across all 21 built-in themes. No hardcoded color.

---

## 8. App Integration

### 8.1 App Struct

Add the status bar to the `App` struct:

```rust
struct App {
    // ... existing fields ...
    status_bar: StatusBar,
}
```

Initialize in `boot()`:

```rust
status_bar: StatusBar::new(),
```

### 8.2 Message Enum

Add to the top-level `Message` enum:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    // ... existing variants ...
    StatusBar(StatusBarMessage),
}
```

### 8.3 Event Handling

Add handler in `App`:

```rust
fn handle_status_bar(&mut self, msg: StatusBarMessage) -> Task<Message> {
    let (task, event) = self.status_bar.update(msg);
    let mut tasks = vec![task.map(Message::StatusBar)];
    if let Some(evt) = event {
        tasks.push(self.handle_status_bar_event(evt));
    }
    Task::batch(tasks)
}

fn handle_status_bar_event(&mut self, event: StatusBarEvent) -> Task<Message> {
    match event {
        StatusBarEvent::RequestReauth { account_id } => {
            // TODO: Open re-authentication flow for this account.
            // This will be wired when the accounts UI is implemented.
            // For now, log the request.
            log::info!("Re-auth requested for account {account_id}");
            Task::none()
        }
    }
}
```

### 8.4 Subscription

Add to `App::subscription()`:

```rust
subs.push(
    self.status_bar.subscription().map(Message::StatusBar),
);
```

### 8.5 View Integration

The status bar sits below all other content. Modify `App::view()` to wrap the existing layout in a `column` with the status bar at the bottom:

```rust
fn view(&self) -> Element<'_, Message> {
    if self.show_settings {
        return self.settings.view().map(Message::Settings);
    }

    // ... existing layout code (sidebar, dividers, thread_list, reading_pane, right_sidebar) ...

    let layout = row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane, right_sidebar]
        .height(Length::Fill);

    let status_bar = self.status_bar.view().map(Message::StatusBar);

    let full_layout = column![layout, status_bar];

    // Wrap in a mouse_area to track drag movement across the full window
    if self.dragging.is_some() {
        mouse_area(full_layout)
            .on_move(Message::DividerDragMove)
            .on_release(Message::DividerDragEnd)
            .interaction(iced::mouse::Interaction::ResizingHorizontally)
            .into()
    } else {
        full_layout.into()
    }
}
```

The status bar occupies a fixed 28px at the bottom. The main panel row gets `Length::Fill` so it takes all remaining space.

---

## 9. Sync Progress Pipeline

### 9.1 Current State

The sync layer (`crates/sync/src/progress.rs`) emits events through the `ProgressReporter` trait. Each provider emits its own event name:

| Provider | Event name |
|---|---|
| Gmail | `gmail-sync-progress` |
| Microsoft Graph | `graph-sync-progress` |
| JMAP | `jmap-sync-progress` |
| IMAP | `imap-sync-progress` |

All share the same JSON payload shape:

```json
{
    "accountId": "...",
    "phase": "messages",
    "current": 1247,
    "total": 8302,
    "folder": "INBOX"
}
```

### 9.2 App-Side ProgressReporter

The iced app will implement `ProgressReporter` to bridge sync events into the iced message loop. This is a channel-based approach:

```rust
/// ProgressReporter implementation that sends events through
/// an iced subscription channel.
pub struct IcedProgressReporter {
    sender: tokio::sync::mpsc::UnboundedSender<SyncEvent>,
}

impl ProgressReporter for IcedProgressReporter {
    fn emit_json(&self, event_name: &str, json: serde_json::Value) {
        let event = SyncEvent::from_json(event_name, json);
        // Best-effort send — drop on failure (receiver closed).
        let _ = self.sender.send(event);
    }
}
```

The corresponding iced subscription reads from the channel receiver and produces `Message::SyncProgress(SyncEvent)` messages.

### 9.3 SyncEvent Type

```rust
#[derive(Debug, Clone)]
pub enum SyncEvent {
    Progress {
        account_id: String,
        phase: String,
        current: u64,
        total: u64,
    },
    Complete {
        account_id: String,
    },
    Error {
        account_id: String,
        error: String,
    },
}
```

### 9.4 App Handling

```rust
// In Message enum:
SyncProgress(SyncEvent),

// In update():
Message::SyncProgress(event) => {
    match event {
        SyncEvent::Progress { account_id, phase, current, total } => {
            let email = self.email_for_account(&account_id);
            self.status_bar.report_sync_progress(
                account_id, email, current, total, phase,
            );
        }
        SyncEvent::Complete { account_id } => {
            self.status_bar.report_sync_complete(&account_id);
        }
        SyncEvent::Error { account_id, error } => {
            let email = self.email_for_account(&account_id);
            self.status_bar.set_warning(AccountWarning {
                account_id,
                email,
                kind: WarningKind::ConnectionFailure { message: error },
            });
        }
    }
    Task::none()
}
```

**Note:** The `email_for_account()` helper looks up the email address from `self.sidebar.accounts` by account ID. This is needed because sync events carry account IDs, but the status bar displays email addresses.

### 9.5 Sync Complete Detection

Sync completion is determined by the sync orchestrator, not the status bar. When the sync task finishes (either successfully or by being cancelled), the app calls `report_sync_complete()`. The status bar does not try to infer completion from progress numbers (e.g., `current == total`) because multi-phase syncs would trigger false completions between phases.

---

## 10. Warning Pipeline

### 10.1 Token Expiry

Token expiry is detected during sync when an OAuth refresh fails. The sync error handler (to be wired in the accounts implementation) catches `TokenExpired` errors and calls:

```rust
self.status_bar.set_warning(AccountWarning {
    account_id: id.clone(),
    email: email.clone(),
    kind: WarningKind::TokenExpiry,
});
```

After successful re-authentication, the accounts flow calls:

```rust
self.status_bar.clear_warning(&account_id);
```

### 10.2 Connection Failure

Persistent connection failures (after retry backoff exhaustion) are surfaced through `SyncEvent::Error`. The app handler (section 9.4) converts these into `ConnectionFailure` warnings.

Connection failures auto-resolve when a subsequent sync succeeds. The sync orchestrator calls `clear_warning()` at the start of a successful sync cycle.

### 10.3 Warning Lifecycle

- Warnings are keyed by `account_id`. Setting a new warning for an account replaces any existing warning for that account.
- `clear_warning()` removes the warning for the given account.
- When a warning is cleared and `cycle_index` exceeds the new warning count, it wraps naturally on the next `resolve()` call (via `%`).

---

## 11. Confirmation Pipeline

### 11.1 Dispatch Points

Confirmations are triggered by the app after successful command execution. Examples:

| Action | Confirmation text |
|---|---|
| Move to Trash | "Message moved to Trash" |
| Archive | "Message archived" |
| Apply label | "Label applied" |
| Remove label | "Label removed" |
| Star | "Message starred" |
| Mark as read | "Marked as read" |
| Snooze | "Snoozed until {time}" |

The app calls `self.status_bar.show_confirmation(text)` from the relevant action handler (e.g., in `handle_thread_list_event` or future command dispatch). This is not routed through the `Message` enum — it is a direct method call in the same `update()` cycle as the action.

### 11.2 Confirmation Preemption

From the problem statement: confirmations briefly interrupt sync progress (~3s), then sync resumes. This is handled by the priority resolution in `resolve()` (section 4): confirmations are checked before sync progress. After `CONFIRMATION_DURATION` elapses, the confirmation is expired and sync progress shows again.

Warnings are never preempted — the `resolve()` function checks warnings first, unconditionally.

### 11.3 Overlapping Confirmations

If a new confirmation arrives while one is already showing, the new one replaces the old one and resets the timer. Only one confirmation is stored at a time. This is correct because the user just performed a new action — the most recent feedback is the most relevant.

---

## 12. Icon Choices

| Content | Icon function | Rationale |
|---|---|---|
| Sync progress | `icon::refresh()` | Rotating arrows convey ongoing activity. Lucide's `refresh-cw` codepoint. |
| Warning | `icon::alert_triangle()` | Standard warning indicator. Already in icon.rs. |
| Confirmation | `icon::check()` | Success/completion. Already in icon.rs. |

All icons use `ICON_MD` (12px) — the standard small icon size. No new icon codepoints are needed.

**Note on animated spinner:** The problem statement mentions a spinner icon (`\u{21BB}`). True rotation animation would require a custom widget with `draw()` override. For the initial implementation, `icon::refresh()` (static) is sufficient. An animated spinner can be added later as a polish pass without changing the component architecture.

---

## 13. Idle State Behavior

When no warnings, sync progress, or confirmations are active, the status bar renders an empty container at its fixed height. This maintains consistent layout — the main content area does not shift when the status bar transitions between idle and active.

The empty container still renders the background and top border, providing a subtle visual baseline at the bottom of the window. This is intentional: the bar's physical presence is constant, only its text content appears and disappears.

---

## 14. Mouse Interaction

Only warning content is interactive. The `view()` function wraps warning content in a `mouse_area` with:

- `on_press(StatusBarMessage::WarningClicked)` — only for `TokenExpiry` warnings (which have a recovery action).
- `interaction(iced::mouse::Interaction::Pointer)` — cursor changes to hand on hover.

`ConnectionFailure` warnings render without the `mouse_area` wrapper (no `on_press`, no cursor change). Sync progress and confirmations are never interactive.

---

## 15. Implementation Phase

This is a single implementation phase. The status bar is small enough to ship as one unit.

### Step 1: Scaffold
- Add `status_bar.rs` module with `StatusBar` struct, all types, `Component` impl.
- Add layout constants (`STATUS_BAR_HEIGHT`, `PAD_STATUS_BAR`).
- Add theme additions (`ContainerClass::StatusBar`, `TextClass::Warning`).
- Wire into `App`: struct field, `boot()` initialization, `Message::StatusBar` variant, `subscription()`, `view()`.
- Verify the empty status bar renders at the bottom of the window.

### Step 2: Confirmations
- Implement `show_confirmation()`.
- Wire a test confirmation from an existing action (e.g., thread selection emits a temporary "Thread selected" confirmation for testing).
- Verify display and auto-expiry.

### Step 3: Sync Progress
- Implement `report_sync_progress()` and `report_sync_complete()`.
- Implement `IcedProgressReporter` and the channel-based subscription.
- Wire to the sync orchestrator (or simulate with a mock timer for testing).
- Verify single-account and multi-account cycling display.

### Step 4: Warnings
- Implement `set_warning()` and `clear_warning()`.
- Wire `WarningClicked` → `StatusBarEvent::RequestReauth`.
- Implement the `handle_status_bar_event` handler in `App`.
- Verify clickable warnings with cursor change.
- Verify multi-warning cycling.

### Step 5: Priority Resolution
- Test all priority combinations:
  - Warning + sync progress (warning wins).
  - Confirmation + sync progress (confirmation briefly wins, then sync resumes).
  - Warning + confirmation (warning wins, confirmation discarded).
  - Warning + sync + confirmation (warning wins).
- Test edge cases: warning cleared while cycling, sync completes during confirmation display.

---

## 16. Testing Strategy

The status bar is pure state + view logic with no DB access and no async operations. Testing is straightforward:

### Unit Tests

```rust
#[test]
fn idle_when_empty() {
    let bar = StatusBar::new();
    let content = bar.resolve(Instant::now());
    assert!(matches!(content, ResolvedContent::Idle));
}

#[test]
fn warning_preempts_sync() {
    let mut bar = StatusBar::new();
    bar.report_sync_progress("a1".into(), "alice@corp.com".into(), 100, 1000, "messages".into());
    bar.set_warning(AccountWarning {
        account_id: "a1".into(),
        email: "alice@corp.com".into(),
        kind: WarningKind::TokenExpiry,
    });
    let content = bar.resolve(Instant::now());
    assert!(matches!(content, ResolvedContent::Warning { .. }));
}

#[test]
fn confirmation_preempts_sync_briefly() {
    let mut bar = StatusBar::new();
    bar.report_sync_progress("a1".into(), "alice@corp.com".into(), 100, 1000, "messages".into());
    bar.show_confirmation("Message moved to Trash".into());

    // Immediately after: confirmation shows.
    let content = bar.resolve(Instant::now());
    assert!(matches!(content, ResolvedContent::Confirmation { .. }));

    // After duration: sync resumes (tested with a synthetic future instant).
}

#[test]
fn warning_never_preempted_by_confirmation() {
    let mut bar = StatusBar::new();
    bar.set_warning(AccountWarning {
        account_id: "a1".into(),
        email: "alice@corp.com".into(),
        kind: WarningKind::TokenExpiry,
    });
    bar.show_confirmation("Label applied".into());
    let content = bar.resolve(Instant::now());
    assert!(matches!(content, ResolvedContent::Warning { .. }));
}
```

### Visual Testing

Manual verification with the seeded database:
1. Trigger confirmations by performing thread actions.
2. Simulate sync progress by calling `report_sync_progress()` from a timer.
3. Simulate warnings by calling `set_warning()` from a debug keyboard shortcut.

---

## 17. Future Extensions

These are explicitly deferred and do not affect the current implementation:

- **Animated spinner icon** — Replace static `refresh` icon with a rotating custom widget during sync.
- **Right-side content** — The problem statement reserves the right side of the status bar for future use (connection indicator, notification count). The current layout uses a single left-aligned row; adding right-side content means switching to a `row![left_content, Space::fill(), right_content]` layout.
- **Undo action on confirmation** — "Message moved to Trash [Undo]" with a clickable undo link. Requires the confirmation struct to carry an undo action closure or message variant.
- **Sync ETA** — "Syncing... ~2 min remaining" based on progress rate. Requires tracking timestamps alongside progress counts.
