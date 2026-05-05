use crate::app::{CHORD_TIMEOUT, ReadyApp};
use crate::command_dispatch::KeyEventMessage;
use crate::handlers;
use crate::message::Message;
use crate::service_subscription::service_notification_subscription;
use crate::ui::settings::SettingsMessage;
use crate::ui::status_bar::sync_progress_subscription;
use crate::{appearance, component::Component};

impl ReadyApp {
    pub(crate) fn subscription(&self) -> iced::Subscription<Message> {
        let mut subs = vec![
            appearance::subscription().map(Message::AppearanceChanged),
            iced::window::resize_events().map(|(id, size)| Message::WindowResized(id, size)),
            iced::window::close_requests().map(Message::WindowCloseRequested),
            iced::event::listen_with(|event, _status, id| {
                if let iced::Event::Window(iced::window::Event::Moved(point)) = event {
                    Some(Message::WindowMoved(id, point))
                } else {
                    None
                }
            }),
            iced::event::listen_with(|event, status, id| match &event {
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key, modifiers, ..
                }) => Some(Message::KeyEvent(KeyEventMessage::KeyPressed {
                    key: key.clone(),
                    modifiers: *modifiers,
                    status,
                    window_id: id,
                })),
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::ModifiersChanged(*modifiers))
                }
                iced::Event::Mouse(iced::mouse::Event::ButtonPressed { .. }) => {
                    // Re-query which filter input owns focus after every
                    // mouse press while settings is open, so the focus
                    // border stays in sync with the actual focused widget.
                    Some(Message::SettingsCheckFocus)
                }
                _ => None,
            }),
            self.sidebar.subscription().map(Message::Sidebar),
            self.thread_list.subscription().map(Message::ThreadList),
            self.reading_pane.subscription().map(Message::ReadingPane),
            self.settings.subscription().map(Message::Settings),
            self.status_bar.subscription().map(Message::StatusBar),
            sync_progress_subscription(&self.sync_receiver).map(Message::SyncProgress),
        ];

        if self.service_client.is_some() {
            subs.push(
                service_notification_subscription(&self.service_notifications)
                    .map(Message::ServiceNotification),
            );
        }

        if self.pending_chord.is_some() {
            subs.push(
                iced::time::every(CHORD_TIMEOUT)
                    .map(|_| Message::KeyEvent(KeyEventMessage::PendingChordTimeout)),
            );
        }

        if let Some(deadline) = self.search_debounce_deadline {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(50))
                    .with(deadline)
                    .map(|(_, deadline)| {
                        if iced::time::Instant::now() >= deadline {
                            Message::SearchExecute
                        } else {
                            Message::Noop
                        }
                    }),
            );
        }

        if self.composer_is_open() && self.has_dirty_compose_drafts() {
            subs.push(
                iced::time::every(handlers::pop_out::DRAFT_AUTO_SAVE_INTERVAL)
                    .map(|_| Message::ComposeDraftTick),
            );
        }

        // Periodic pinned search expiry - check every hour
        if !self.pinned_searches.is_empty() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(3600))
                    .map(|_| Message::ExpiryTick),
            );
        }

        // Periodic sync - delta sync all accounts every 5 minutes
        if !self.sidebar.accounts.is_empty() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(300)).map(|_| Message::SyncTick),
            );
        }

        // Snooze resurface - check every 60 seconds for due threads
        if self.action_ctx.is_some() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(60)).map(|_| Message::SnoozeTick),
            );
        }

        // Phase 5 task 10: the dedicated 1-hour `Message::GalRefreshTick`
        // subscription is gone. GAL refresh now rides on the 5-min
        // `SyncTick` via `Message::SyncTick -> kick_gal_refresh`, gated
        // Service-side by the existing 24 h cache check inside
        // `refresh_gal_for_account`. Two cadences would mean two failure
        // modes (one timer stops working but the other doesn't); one
        // cadence + Service-side gating is the simpler shape and
        // survives the Phase 9 tray-resident move unchanged.

        // Phase 3 task 17: debounced reader reload after
        // `index.committed` notifications. Polls every 200 ms; the
        // handler reloads the searcher only when there is a pending
        // reload that has aged at least one tick. Cheap when idle (no
        // pending) and bounded under heavy initial-sync pressure
        // (~5 reloads/sec at most).
        if self.search_state.is_some() {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(200))
                    .map(|_| Message::ReaderReloadTick),
            );
        }

        // Phase 5 task 11: debounced calendar reload after CalendarChanged
        // notifications. Polls every 250 ms; the handler reloads the
        // calendar tab only when there is a pending stamp that has aged at
        // least one tick. Collapses an N-account kick batch (one
        // CalendarChanged per stale account) into a single reload.
        subs.push(
            iced::time::every(std::time::Duration::from_millis(250))
                .map(|_| Message::CalendarReloadTick),
        );

        if self
            .settings
            .sheet_anim
            .is_animating(iced::time::Instant::now())
        {
            subs.push(
                iced::window::frames()
                    .map(|at| Message::Settings(SettingsMessage::SheetAnimTick(at))),
            );
        }

        iced::Subscription::batch(subs)
    }
}
