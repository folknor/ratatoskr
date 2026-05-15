use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use iced::advanced::graphics::futures::subscription;
use iced::advanced::subscription::Hasher;
use iced::futures::StreamExt;
use iced::futures::stream::BoxStream;
use iced::widget::{Space, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, Subscription};

use crate::app::RebuildProgressState;
use crate::component::Component;
use crate::icon;
use crate::service_client::ServiceHealth;
use crate::ui::layout::*;
use crate::ui::theme::{ContainerClass, TextClass};

/// How long transient confirmations are visible.
const CONFIRMATION_DURATION: std::time::Duration = std::time::Duration::from_secs(3);

/// Minimum interval between back-to-back "New mail in <account>"
/// confirmations for the same account. Without this rate limit, a
/// heavy import bursting StateChanges every debounce window would
/// surface a fresh confirmation every ~500 ms, replacing the prior
/// one and producing flicker. 60 s strikes a balance: the user gets
/// notified per fresh push burst, but a sustained import shows one
/// confirmation per minute rather than per second.
const PUSH_CONFIRMATION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

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
    /// OAuth token expired or refresh failed. Clickable - opens re-auth.
    TokenExpiry,
    /// Persistent connection failure. Not clickable (no recovery action).
    ConnectionFailure { message: String },
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // phase + generation populated for the upcoming staleness pruning
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
#[allow(dead_code)] // account_id needed once the warning click handler dispatches per-account
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
    ServiceHealth {
        text: String,
        class: TextClass,
    },
    RebuildProgress {
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
        // Best-effort send - drop on failure (receiver closed).
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
    /// Per-account timestamp of the most recent JMAP push event. Set
    /// from `Notification::PushEvent` arrivals; surfaced as a "new
    /// mail arrived" indicator. Coalesce semantics on the wire mean
    /// the entry always reflects the latest event for the account.
    last_push_at: HashMap<String, std::time::Instant>,
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
            last_push_at: HashMap::new(),
        }
    }

    /// Record a JMAP push event for `account_id`.
    ///
    /// Stamps the most-recent-push timestamp on the
    /// `last_push_at[account_id]` entry (reserved for a future
    /// per-account icon indicator) and, if the previous push for
    /// the same account was more than `PUSH_CONFIRMATION_INTERVAL`
    /// ago, fires a brief "New mail in <label>" confirmation. The
    /// rate-limit prevents a heavy import (StateChanges arriving
    /// every debounce window) from spamming the status bar with
    /// identical confirmations.
    ///
    /// `account_label` is the display string surfaced to the user
    /// (typically the account email); the caller resolves it from
    /// the sidebar account list.
    pub fn record_push_event(&mut self, account_id: String, account_label: &str) {
        let now = std::time::Instant::now();
        let should_confirm = self
            .last_push_at
            .get(&account_id)
            .is_none_or(|prev| now.duration_since(*prev) >= PUSH_CONFIRMATION_INTERVAL);
        self.last_push_at.insert(account_id, now);
        if should_confirm {
            self.show_confirmation(format!("New mail in {account_label}"));
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
    #[allow(dead_code)] // generation pruning landing in a follow-up
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
    #[allow(dead_code)] // generation pruning landing in a follow-up
    pub fn is_sync_stale(&self, account_id: &str) -> bool {
        let Some(progress) = self.sync_progress.get(account_id) else {
            return false;
        };
        let current_gen = self.sync_generations.get(account_id).copied().unwrap_or(0);
        progress.generation != current_gen
    }

    /// Remove stale sync progress entries (where generation is behind).
    #[allow(dead_code)] // generation pruning landing in a follow-up
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

    fn resolve(
        &self,
        now: iced::time::Instant,
        service_health: &ServiceHealth,
        rebuild: Option<&RebuildProgressState>,
    ) -> ResolvedContent {
        if let Some(content) = resolve_service_health(service_health) {
            return content;
        }

        // 1. Warnings win over normal activity.
        if !self.warnings.is_empty() {
            return self.resolve_warning();
        }

        // 2. Rebuild progress is long-running system work.
        if let Some(rebuild) = rebuild {
            return resolve_rebuild_progress(rebuild);
        }

        // 3. Active confirmation briefly preempts sync progress.
        if let Some(ref conf) = self.confirmation
            && now.duration_since(conf.created_at) < CONFIRMATION_DURATION
        {
            return ResolvedContent::Confirmation {
                text: conf.text.clone(),
            };
        }

        // 4. Sync progress is the steady-state default.
        if !self.sync_progress.is_empty() {
            return self.resolve_sync_progress();
        }

        // 5. Confirmation that arrived with no sync active.
        if let Some(ref conf) = self.confirmation
            && now.duration_since(conf.created_at) < CONFIRMATION_DURATION
        {
            return ResolvedContent::Confirmation {
                text: conf.text.clone(),
            };
        }

        // 6. Persistent auto-reply indicator (lowest priority above idle).
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

    pub fn view_with_system_status(
        &self,
        service_health: &ServiceHealth,
        rebuild: Option<&RebuildProgressState>,
    ) -> Element<'_, StatusBarMessage> {
        self.view_resolved(service_health, rebuild)
    }

    fn view_resolved(
        &self,
        service_health: &ServiceHealth,
        rebuild: Option<&RebuildProgressState>,
    ) -> Element<'_, StatusBarMessage> {
        let now = iced::time::Instant::now();
        let content = self.resolve(now, service_health, rebuild);

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
                    bar
                }
            }
            ResolvedContent::SyncProgress { text: sync_text } => {
                build_status_row(icon::refresh(), &sync_text, TextClass::Muted)
            }
            ResolvedContent::Confirmation { text: conf_text } => {
                build_status_row(icon::check(), &conf_text, TextClass::Muted)
            }
            ResolvedContent::ServiceHealth { text, class } => {
                build_status_row(icon::alert_triangle(), &text, class)
            }
            ResolvedContent::RebuildProgress { text } => {
                build_status_row(icon::refresh(), &text, TextClass::Accent)
            }
            ResolvedContent::AutoReplyActive => build_status_row(
                icon::mail(),
                "Out of Office auto-reply is active",
                TextClass::Accent,
            ),
        }
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

                if let Some(ref conf) = self.confirmation
                    && now.duration_since(conf.created_at) >= CONFIRMATION_DURATION
                {
                    self.confirmation = None;
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
        self.view_resolved(&ServiceHealth::Healthy, None)
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
    let text_styled = text(label.to_string())
        .size(TEXT_SM)
        .style(class.style());

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

fn resolve_service_health(health: &ServiceHealth) -> Option<ResolvedContent> {
    match health {
        ServiceHealth::Healthy => None,
        ServiceHealth::Booting { phase } => Some(ResolvedContent::ServiceHealth {
            text: format!("Service starting: {phase}"),
            class: TextClass::Muted,
        }),
        ServiceHealth::Respawning {
            attempt,
            next_delay,
        } => Some(ResolvedContent::ServiceHealth {
            text: format!(
                "Service respawning after crash (attempt {}, retry in {}s)",
                attempt,
                next_delay.as_secs().max(1),
            ),
            class: TextClass::Warning,
        }),
        ServiceHealth::PersistentlyFailing { reason } => Some(ResolvedContent::ServiceHealth {
            text: format!("Service cannot restart: {reason}"),
            class: TextClass::Warning,
        }),
    }
}

fn resolve_rebuild_progress(rebuild: &RebuildProgressState) -> ResolvedContent {
    let text = if rebuild.total == 0 {
        "Rebuilding search index".to_string()
    } else {
        format!(
            "Rebuilding search index ({} / {})",
            format_number(rebuild.processed),
            format_number(rebuild.total),
        )
    };
    ResolvedContent::RebuildProgress { text }
}
