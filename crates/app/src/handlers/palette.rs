use iced::Task;

use crate::command_dispatch;
use crate::component::Component;
use crate::ui::palette::{PaletteEvent, PaletteMessage};
use crate::{App, Message};

impl App {
    /// Dispatch a palette message through the component and handle events.
    ///
    /// Before dispatching, injects the current `CommandContext` into message
    /// variants that carry a placeholder context (from the view closures).
    pub(crate) fn handle_palette(&mut self, msg: PaletteMessage) -> Task<Message> {
        // Dismiss conflicting overlays when opening the palette
        if matches!(msg, PaletteMessage::Open(_)) {
            self.dismiss_overlays();
        }

        // Inject the current command context into messages that need it
        let msg = self.inject_palette_context(msg);

        let (task, event) = self.palette.update(msg);
        let mut tasks = vec![task.map(Message::Palette)];
        if let Some(evt) = event {
            tasks.push(self.handle_palette_event(evt));
        }
        Task::batch(tasks)
    }

    /// Replace placeholder `CommandContext::default()` with the real context.
    fn inject_palette_context(&self, msg: PaletteMessage) -> PaletteMessage {
        let ctx = || command_dispatch::build_context(self);
        match msg {
            PaletteMessage::Open(_) => PaletteMessage::Open(ctx()),
            PaletteMessage::Close(_) => PaletteMessage::Close(ctx()),
            PaletteMessage::QueryChanged(q, _) => PaletteMessage::QueryChanged(q, ctx()),
            PaletteMessage::Confirm(_) => PaletteMessage::Confirm(ctx()),
            PaletteMessage::ClickResult(idx, _) => PaletteMessage::ClickResult(idx, ctx()),
            // These don't need context injection
            other @ (PaletteMessage::SelectNext
            | PaletteMessage::SelectPrev
            | PaletteMessage::ClickOption(_)
            | PaletteMessage::OptionsLoaded(..)) => other,
        }
    }

    /// Handle events emitted by the palette component.
    fn handle_palette_event(&mut self, event: PaletteEvent) -> Task<Message> {
        match event {
            PaletteEvent::ExecuteCommand(id) => self.update(Message::ExecuteCommand(id)),
            PaletteEvent::ExecuteParameterized(id, args) => {
                self.update(Message::ExecuteParameterized(id, args))
            }
            PaletteEvent::Dismissed => Task::none(),
            PaletteEvent::Error(msg) => {
                self.status = format!("Palette error: {msg}");
                Task::none()
            }
        }
    }
}
