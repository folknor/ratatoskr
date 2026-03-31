use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::StreamExt;
use iced::futures::stream::BoxStream;
use iced::widget::{Space, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, Subscription};

use crate::component::Component;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme::{ContainerClass, TextClass};

/// How long transient confirmations are visible.
const CONFIRMATION_DURATION: std::time::Duration = std::time::Duration::from_secs(3);

/// Cycle interval for rotating through multiple warnings or syncing accounts.
const CYCLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

// ── Types ───────────────────────────────────────────────

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

#[derive(Debug, Clone)]
pub struct SyncAccountProgress {
    pub email: String,
    pub current: u64,
    pub total: u64,
    pub phase: String,
    /// Generation counter. Incremented each time a sync cycle starts for
    /// this account. Used to detect stale progress entries from dead tasks.
    pub generation: u64,
}

#[derive(Debug, Clone)]
struct Confirmation {
    text: String,
    created_at: iced::time::Instant,
}

/// Resolved display content, computed fresh on every `view()` call.
enum ResolvedContent {
    Idle,
    Warning {
        text: String,
        clickable: bool,
        account_id: String,
    },
    SyncProgress {
        text: String,
    },
    Confirmation {
        text: String,
    },
    /// At least one account has an active auto-reply / out-of-office.
    AutoReplyActive,
}

// ── Sync event pipeline ─────────────────────────────────

/// Events produced by the `IcedProgressReporter` and delivered to the
/// app via an iced subscription channel.
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

impl SyncEvent {
    /// Parse a `SyncEvent` from a raw progress reporter event name and
    /// JSON payload.
    pub fn from_json(event_name: &str, json: &serde_json::Value) -> Self {
        // Sync-complete events are signalled by the event name suffix.
        if event_name.ends_with("-sync-complete") {
            let account_id = json
                .get("accountId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            return Self::Complete { account_id };
        }

        // Error events.
        if event_name.ends_with("-sync-error") {
            let account_id = json
                .get("accountId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let error = json
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown error")
                .to_string();
            return Self::Error { account_id, error };
        }

        // Default: progress event.
        let account_id = json
            .get("accountId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let phase = json
            .get("phase")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let current = json
            .get("current")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let total = json
            .get("total")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        Self::Progress {
            account_id,
            phase,
            current,
            total,
        }
    }
}

/// `ProgressReporter` implementation that sends events through an iced
/// subscription channel. Created via `sync_progress_subscription()`.
pub struct IcedProgressReporter {
    sender: tokio::sync::mpsc::UnboundedSender<SyncEvent>,
}

impl rtsk::progress::ProgressReporter for IcedProgressReporter {
    fn emit_json(&self, event_name: &str, json: serde_json::Value) {
        let event = SyncEvent::from_json(event_name, &json);
        // Best-effort send — drop on failure (receiver closed).
        let _ = self.sender.send(event);
    }
}

/// Create a sync progress subscription and its corresponding reporter.
///
/// Returns `(subscription, reporter)`. The subscription produces
/// `SyncEvent` messages. The reporter should be passed to the sync
/// layer as its `ProgressReporter`.
pub fn create_sync_progress_channel() -> (
    tokio::sync::mpsc::UnboundedReceiver<SyncEvent>,
    IcedProgressReporter,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    (receiver, IcedProgressReporter { sender })
}

/// Shared handle for the sync progress receiver. The subscription
/// recipe takes the receiver out on first poll; subsequent calls
/// to the recipe (iced may re-create it) find `None` and produce
/// an empty stream.
pub type SyncProgressReceiver = Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<SyncEvent>>>>;

/// Create a new shared receiver handle from a raw receiver.
pub fn shared_receiver(
    rx: tokio::sync::mpsc::UnboundedReceiver<SyncEvent>,
) -> SyncProgressReceiver {
    Arc::new(Mutex::new(Some(rx)))
}

/// iced subscription that drains the sync progress channel and
/// yields `SyncEvent` messages.
struct SyncProgressRecipe {
    receiver: SyncProgressReceiver,
}

impl subscription::Recipe for SyncProgressRecipe {
    type Output = SyncEvent;

    fn hash(&self, state: &mut Hasher) {
        use std::hash::Hash;
        struct Marker;
        std::any::TypeId::of::<Marker>().hash(state);
    }

    fn stream(self: Box<Self>, _input: subscription::EventStream) -> BoxStream<'static, SyncEvent> {
        let taken = self.receiver.lock().ok().and_then(|mut guard| guard.take());

        match taken {
            Some(rx) => iced::futures::stream::unfold(rx, |mut rx| async {
                let event = rx.recv().await?;
                Some((event, rx))
            })
            .boxed(),
            None => iced::futures::stream::empty().boxed(),
        }
    }
}

/// Build an iced `Subscription` that yields `SyncEvent` from the
/// shared receiver.
pub fn sync_progress_subscription(receiver: &SyncProgressReceiver) -> Subscription<SyncEvent> {
    subscription::from_recipe(SyncProgressRecipe {
        receiver: Arc::clone(receiver),
    })
}

// ── Messages & Events ───────────────────────────────────

#[derive(Debug, Clone)]
pub enum StatusBarMessage {
    /// Timer tick for cycling and expiring confirmations.
    CycleTick(iced::time::Instant),
    /// User clicked a clickable warning.
    WarningClicked,
}

#[derive(Debug, Clone)]
pub enum StatusBarEvent {
    /// User clicked a token expiry warning.
    RequestReauth { account_id: String },
}

// ── State ───────────────────────────────────────────────

pub struct StatusBar {
    warnings: BTreeMap<String, AccountWarning>,
    sync_progress: HashMap<String, SyncAccountProgress>,
    confirmation: Option<Confirmation>,
    warning_cycle_index: usize,
    sync_cycle_index: usize,
    /// Per-account generation counters for stale sync detection.
    /// Incremented when a sync cycle starts; if a progress entry's
    /// generation is behind the current generation, the entry is stale.
    sync_generations: HashMap<String, u64>,
    /// True when any account has an active auto-reply / out-of-office.
    auto_reply_active: bool,
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            warnings: BTreeMap::new(),
            sync_progress: HashMap::new(),
            confirmation: None,
            warning_cycle_index: 0,
            sync_cycle_index: 0,
            sync_generations: HashMap::new(),
            auto_reply_active: false,
        }
    }

    // ── Inbound data methods (called by App) ────────────

    /// Record sync progress for an account. Called when the progress
    /// reporter delivers a Progress event.
    pub fn report_sync_progress(
        &mut self,
        account_id: String,
        email: String,
        current: u64,
        total: u64,
        phase: String,
    ) {
        let generation = self.current_generation(&account_id);
        self.sync_progress.insert(
            account_id,
            SyncAccountProgress {
                email,
                current,
                total,
                phase,
                generation,
            },
        );
    }

    /// Remove sync progress for an account (sync finished or cancelled).
    pub fn report_sync_complete(&mut self, account_id: &str) {
        self.sync_progress.remove(account_id);
    }

    /// Begin a new sync generation for an account. Returns the new
    /// generation number. Call this at the start of each sync cycle
    /// so that old progress entries can be detected as stale.
    pub fn begin_sync_generation(&mut self, account_id: &str) -> u64 {
        let generation = self
            .sync_generations
            .entry(account_id.to_string())
            .or_insert(0);
        *generation = generation.wrapping_add(1);
        *generation
    }

    /// Check if an account's sync progress entry is stale (its
    /// generation is behind the current generation for that account).
    pub fn is_sync_stale(&self, account_id: &str) -> bool {
        let Some(progress) = self.sync_progress.get(account_id) else {
            return false;
        };
        let current_gen = self.sync_generations.get(account_id).copied().unwrap_or(0);
        progress.generation != current_gen
    }

    /// Remove stale sync progress entries (where generation is behind).
    pub fn prune_stale_sync(&mut self) {
        let stale_ids: Vec<String> = self
            .sync_progress
            .keys()
            .filter(|id| self.is_sync_stale(id))
            .cloned()
            .collect();
        for id in stale_ids {
            self.sync_progress.remove(&id);
        }
    }

    fn current_generation(&self, account_id: &str) -> u64 {
        self.sync_generations.get(account_id).copied().unwrap_or(0)
    }

    /// Set a warning for an account. Replaces any existing warning.
    pub fn set_warning(&mut self, warning: AccountWarning) {
        self.warnings.insert(warning.account_id.clone(), warning);
    }

    /// Clear the warning for an account.
    pub fn clear_warning(&mut self, account_id: &str) {
        self.warnings.remove(account_id);
    }

    /// Show a transient confirmation message (~3s).
    pub fn show_confirmation(&mut self, text: String) {
        self.confirmation = Some(Confirmation {
            text,
            created_at: iced::time::Instant::now(),
        });
    }

    /// Update whether any account has an active auto-reply.
    pub fn set_auto_reply_active(&mut self, active: bool) {
        self.auto_reply_active = active;
    }

    // ── Priority resolution ─────────────────────────────

    fn resolve(&self, now: iced::time::Instant) -> ResolvedContent {
        // 1. Warnings always win.
        if !self.warnings.is_empty() {
            return self.resolve_warning();
        }

        // 2. Active confirmation briefly preempts sync progress.
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

        // 5. Persistent auto-reply indicator (lowest priority above idle).
        if self.auto_reply_active {
            return ResolvedContent::AutoReplyActive;
        }

        ResolvedContent::Idle
    }

    fn resolve_warning(&self) -> ResolvedContent {
        let warnings: Vec<&AccountWarning> = self.warnings.values().collect();
        let idx = self.warning_cycle_index % warnings.len();
        let warning = warnings[idx];
        let clickable = matches!(warning.kind, WarningKind::TokenExpiry);

        let text = if warnings.len() == 1 {
            match &warning.kind {
                WarningKind::TokenExpiry => {
                    format!(
                        "{} needs re-authentication \u{2014} click to sign in",
                        warning.email,
                    )
                }
                WarningKind::ConnectionFailure { message } => {
                    format!("{} \u{2014} connection failed ({})", warning.email, message,)
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
            format!(
                "{} accounts need attention \u{2014} {}",
                warnings.len(),
                detail,
            )
        };

        ResolvedContent::Warning {
            text,
            clickable,
            account_id: warning.account_id.clone(),
        }
    }

    fn resolve_sync_progress(&self) -> ResolvedContent {
        let accounts: Vec<&SyncAccountProgress> = self.sync_progress.values().collect();

        let text = if accounts.len() == 1 {
            let p = accounts[0];
            format!(
                "Syncing {} ({} / {})",
                p.email,
                format_number(p.current),
                format_number(p.total),
            )
        } else {
            let idx = self.sync_cycle_index % accounts.len();
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
}

// ── Component implementation ────────────────────────────

impl Component for StatusBar {
    type Message = StatusBarMessage;
    type Event = StatusBarEvent;

    fn update(
        &mut self,
        message: StatusBarMessage,
    ) -> (iced::Task<StatusBarMessage>, Option<StatusBarEvent>) {
        match message {
            StatusBarMessage::CycleTick(now) => {
                self.warning_cycle_index = self.warning_cycle_index.wrapping_add(1);
                self.sync_cycle_index = self.sync_cycle_index.wrapping_add(1);

                if let Some(ref conf) = self.confirmation {
                    if now.duration_since(conf.created_at) >= CONFIRMATION_DURATION {
                        self.confirmation = None;
                    }
                }

                (iced::Task::none(), None)
            }
            StatusBarMessage::WarningClicked => {
                if self.warnings.is_empty() {
                    return (iced::Task::none(), None);
                }
                let warnings: Vec<&AccountWarning> = self.warnings.values().collect();
                let idx = self.warning_cycle_index % warnings.len();
                let warning = warnings[idx];
                match warning.kind {
                    WarningKind::TokenExpiry => {
                        let event = StatusBarEvent::RequestReauth {
                            account_id: warning.account_id.clone(),
                        };
                        (iced::Task::none(), Some(event))
                    }
                    WarningKind::ConnectionFailure { .. } => (iced::Task::none(), None),
                }
            }
        }
    }

    fn view(&self) -> Element<'_, StatusBarMessage> {
        let now = iced::time::Instant::now();
        let content = self.resolve(now);

        match content {
            ResolvedContent::Idle => {
                // Empty bar at fixed height for consistent layout.
                container(Space::new().height(0))
                    .width(Length::Fill)
                    .height(STATUS_BAR_HEIGHT)
                    .style(ContainerClass::StatusBar.style())
                    .into()
            }
            ResolvedContent::Warning {
                text: warning_text,
                clickable,
                ..
            } => {
                let bar =
                    build_status_row(icon::alert_triangle(), &warning_text, TextClass::Warning);

                if clickable {
                    mouse_area(bar)
                        .on_press(StatusBarMessage::WarningClicked)
                        .interaction(iced::mouse::Interaction::Pointer)
                        .into()
                } else {
                    bar.into()
                }
            }
            ResolvedContent::SyncProgress { text: sync_text } => {
                build_status_row(icon::refresh(), &sync_text, TextClass::Muted).into()
            }
            ResolvedContent::Confirmation { text: conf_text } => {
                build_status_row(icon::check(), &conf_text, TextClass::Muted).into()
            }
            ResolvedContent::AutoReplyActive => build_status_row(
                icon::mail(),
                "Out of Office auto-reply is active",
                TextClass::Accent,
            )
            .into(),
        }
    }

    fn subscription(&self) -> iced::Subscription<StatusBarMessage> {
        let needs_tick =
            self.warnings.len() > 1 || self.sync_progress.len() > 1 || self.confirmation.is_some();

        if needs_tick {
            iced::time::every(CYCLE_INTERVAL).map(StatusBarMessage::CycleTick)
        } else {
            iced::Subscription::none()
        }
    }
}

// ── View helpers ────────────────────────────────────────

fn build_status_row<'a>(
    icon_el: iced::widget::Text<'a>,
    label: &str,
    class: TextClass,
) -> Element<'a, StatusBarMessage> {
    let icon_styled = icon_el.size(ICON_MD).style(class.style());
    let text_styled = text(label.to_string()).size(TEXT_SM).style(class.style());

    let content_row = row![
        container(icon_styled).align_y(Alignment::Center),
        container(text_styled).align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    container(content_row)
        .padding(PAD_STATUS_BAR)
        .width(Length::Fill)
        .height(STATUS_BAR_HEIGHT)
        .style(ContainerClass::StatusBar.style())
        .into()
}

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
