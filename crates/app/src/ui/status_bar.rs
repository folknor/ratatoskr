use std::collections::{BTreeMap, HashMap};

use iced::widget::{container, mouse_area, row, text, Space};
use iced::{Alignment, Element, Length};

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
    },
    SyncProgress {
        text: String,
    },
    Confirmation {
        text: String,
    },
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
}

impl StatusBar {
    pub fn new() -> Self {
        Self {
            warnings: BTreeMap::new(),
            sync_progress: HashMap::new(),
            confirmation: None,
            warning_cycle_index: 0,
            sync_cycle_index: 0,
        }
    }

    // ── Inbound data methods (called by App) ────────────

    pub fn report_sync_progress(
        &mut self,
        account_id: String,
        email: String,
        current: u64,
        total: u64,
        phase: String,
    ) {
        self.sync_progress.insert(
            account_id,
            SyncAccountProgress { email, current, total, phase },
        );
    }

    pub fn report_sync_complete(&mut self, account_id: &str) {
        self.sync_progress.remove(account_id);
    }

    pub fn set_warning(&mut self, warning: AccountWarning) {
        self.warnings.insert(warning.account_id.clone(), warning);
    }

    pub fn clear_warning(&mut self, account_id: &str) {
        self.warnings.remove(account_id);
    }

    pub fn show_confirmation(&mut self, text: String) {
        self.confirmation = Some(Confirmation {
            text,
            created_at: iced::time::Instant::now(),
        });
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
                    format!(
                        "{} \u{2014} connection failed ({})",
                        warning.email, message,
                    )
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

        ResolvedContent::Warning { text, clickable }
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
                    WarningKind::ConnectionFailure { .. } => {
                        (iced::Task::none(), None)
                    }
                }
            }
        }
    }

    fn view(&self) -> Element<'_, StatusBarMessage> {
        let now = iced::time::Instant::now();
        let content = self.resolve(now);

        match content {
            ResolvedContent::Idle => {
                // Nothing to show — collapse to zero height.
                // "Absence means nothing to say" per the problem statement.
                Space::new().width(0).height(0).into()
            }
            ResolvedContent::Warning {
                text: warning_text,
                clickable,
            } => {
                let bar = build_status_row(
                    icon::alert_triangle(),
                    &warning_text,
                    TextClass::Warning,
                );

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
                build_status_row(
                    icon::refresh(),
                    &sync_text,
                    TextClass::Muted,
                )
                .into()
            }
            ResolvedContent::Confirmation { text: conf_text } => {
                build_status_row(
                    icon::check(),
                    &conf_text,
                    TextClass::Muted,
                )
                .into()
            }
        }
    }

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
