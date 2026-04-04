use iced::Task;

use crate::command_dispatch::{self, KeyEventMessage};
use crate::pop_out::PopOutWindow;
use crate::ui::thread_list::{ThreadListMessage, TypeaheadDirection};
use crate::{App, Message, PendingChord};
use cmdk::{Chord, CommandId, ResolveResult};

impl App {
    pub(crate) fn handle_key_event(&mut self, msg: KeyEventMessage) -> Task<Message> {
        match msg {
            KeyEventMessage::KeyPressed {
                key,
                modifiers,
                status,
                window_id,
            } => {
                if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                    && window_id != self.main_window_id
                {
                    if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                    {
                        if state.link_dialog_open {
                            state.link_dialog_open = false;
                            state.link_url.clear();
                            state.link_text.clear();
                            return Task::none();
                        }
                        if state.discard_confirm_open {
                            state.discard_confirm_open = false;
                            return Task::none();
                        }
                    }
                }

                // Escape in a pop-out window closes that window
                if key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                    && window_id != self.main_window_id
                    && self.pop_out_windows.contains_key(&window_id)
                {
                    return self.handle_window_close(window_id);
                }
                self.handle_key_pressed(&key, modifiers, status)
            }
            KeyEventMessage::PendingChordTimeout => {
                self.pending_chord = None;
                Task::none()
            }
        }
    }

    fn handle_key_pressed(
        &mut self,
        key: &iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        status: iced::event::Status,
    ) -> Task<Message> {
        // 1. If palette is open, route to palette-specific handler
        if self.palette.is_open() {
            return self.handle_palette_key(key);
        }

        // 2. If settings is open, Escape closes it.
        if self.show_settings {
            if *key == iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) {
                return self.update(Message::Settings(
                    crate::ui::settings::SettingsMessage::Close,
                ));
            }
            // Don't process other shortcuts while settings is open.
            return Task::none();
        }

        // 2a. If typeahead is visible, intercept arrow keys and Tab/Enter/Escape.
        if self.thread_list.typeahead.visible {
            match key {
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadNavigate(
                        TypeaheadDirection::Up,
                    )));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadNavigate(
                        TypeaheadDirection::Down,
                    )));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab) => {
                    // Tab accepts the selected typeahead item.
                    let idx = self.thread_list.typeahead.selected;
                    return self
                        .update(Message::ThreadList(ThreadListMessage::TypeaheadSelect(idx)));
                }
                iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                    return self.update(Message::ThreadList(ThreadListMessage::TypeaheadDismiss));
                }
                _ => {}
            }
        }

        // 2b. If a text input or other widget captured the event, skip
        //     (unless it's a modifier-chord like Ctrl+K)
        if status == iced::event::Status::Captured
            && !command_dispatch::has_command_modifier(&modifiers)
        {
            return Task::none();
        }

        // 3. Convert iced key to cmdk Chord
        let Some(chord) = command_dispatch::iced_key_to_chord(key, &modifiers) else {
            return Task::none();
        };

        // 4. If we're in pending chord state, resolve the sequence
        if let Some(pending) = self.pending_chord.take() {
            log::debug!(
                "Resolving chord sequence: {:?} + {:?}",
                pending.first,
                chord
            );
            if let Some(id) = self.binding_table.resolve_sequence(&pending.first, &chord) {
                if self.is_command_available(id) {
                    return self.update(Message::ExecuteCommand(id));
                }
                return Task::none();
            }
            // Second chord didn't match any sequence — re-process as fresh first chord
            return self.try_resolve_single_chord(chord);
        }

        // 5. Resolve single chord
        self.try_resolve_single_chord(chord)
    }

    /// Check if a command is currently available given app context.
    pub(crate) fn is_command_available(&self, id: CommandId) -> bool {
        let ctx = command_dispatch::build_context(self);
        self.registry
            .get(id)
            .is_some_and(|desc| (desc.is_available)(&ctx))
    }

    /// When the palette is open, intercept Escape/ArrowUp/ArrowDown/Enter.
    fn handle_palette_key(&mut self, key: &iced::keyboard::Key) -> Task<Message> {
        use crate::ui::palette::PaletteMessage;
        let default_ctx = cmdk::CommandContext::default();
        match key {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                self.update(Message::Palette(PaletteMessage::Close(default_ctx)))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                self.update(Message::Palette(PaletteMessage::SelectNext))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                self.update(Message::Palette(PaletteMessage::SelectPrev))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
                self.update(Message::Palette(PaletteMessage::Confirm(default_ctx)))
            }
            _ => Task::none(),
        }
    }

    /// Try to resolve a single chord, checking availability before dispatch.
    fn try_resolve_single_chord(&mut self, chord: Chord) -> Task<Message> {
        match self.binding_table.resolve_chord(&chord) {
            ResolveResult::Command(id) => {
                if self.is_command_available(id) {
                    self.update(Message::ExecuteCommand(id))
                } else {
                    Task::none()
                }
            }
            ResolveResult::Pending => {
                self.pending_chord = Some(PendingChord { first: chord });
                Task::none()
            }
            ResolveResult::NoMatch => Task::none(),
        }
    }
}
