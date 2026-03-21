use std::sync::Arc;

use iced::Task;

use crate::command_dispatch::{self, PaletteMessage};
use crate::ui::palette::PaletteStage;
use crate::{App, Message};
use ratatoskr_command_palette::{CommandId, CommandInputResolver};

impl App {
    pub(crate) fn handle_palette(&mut self, msg: PaletteMessage) -> Task<Message> {
        match msg {
            PaletteMessage::Open => {
                // Don't open palette when settings overlay is showing —
                // the palette can't render there, creating a hidden-modal state.
                if self.show_settings {
                    return Task::none();
                }
                let ctx = command_dispatch::build_context(self);
                let results = self.registry.query(&ctx, "");
                self.palette.open = true;
                self.palette.query.clear();
                self.palette.results = results;
                self.palette.selected_index = 0;
                self.palette.stage = PaletteStage::CommandSearch;
                iced::widget::operation::focus::<Message>("palette-input".to_string())
            }
            PaletteMessage::Close => {
                // In stage 2, Escape goes back to stage 1 instead of closing.
                if self.palette.is_option_pick() {
                    let ctx = command_dispatch::build_context(self);
                    self.palette.back_to_stage1();
                    self.palette.results = self.registry.query(&ctx, "");
                    return iced::widget::operation::focus::<Message>(
                        "palette-input".to_string(),
                    );
                }
                self.palette.close();
                Task::none()
            }
            PaletteMessage::QueryChanged(query) => {
                if self.palette.is_option_pick() {
                    // Stage 2: filter options with fuzzy search
                    self.palette.option_matches = ratatoskr_command_palette::search_options(
                        &self.palette.option_items,
                        &query,
                    );
                    self.palette.query = query;
                    self.palette.selected_index = 0;
                } else {
                    // Stage 1: query the registry
                    let ctx = command_dispatch::build_context(self);
                    self.palette.results = self.registry.query(&ctx, &query);
                    self.palette.query = query;
                    self.palette.selected_index = 0;
                }
                Task::none()
            }
            PaletteMessage::SelectNext => {
                let len = if self.palette.is_option_pick() {
                    self.palette.option_matches.len()
                } else {
                    self.palette.results.len()
                };
                if len > 0 {
                    self.palette.selected_index = (self.palette.selected_index + 1)
                        .min(len - 1);
                }
                Task::none()
            }
            PaletteMessage::SelectPrev => {
                self.palette.selected_index = self.palette.selected_index.saturating_sub(1);
                Task::none()
            }
            PaletteMessage::Confirm => {
                if self.palette.is_option_pick() {
                    self.palette_confirm_option()
                } else {
                    self.palette_confirm()
                }
            }
            PaletteMessage::ClickResult(idx) => {
                if idx < self.palette.results.len() {
                    self.palette.selected_index = idx;
                    self.palette_confirm()
                } else {
                    Task::none()
                }
            }
            PaletteMessage::ClickOption(idx) => {
                if idx < self.palette.option_matches.len() {
                    self.palette.selected_index = idx;
                    self.palette_confirm_option()
                } else {
                    Task::none()
                }
            }
            PaletteMessage::OptionsLoaded(generation, command_id, result) => {
                self.handle_options_loaded(generation, command_id, result)
            }
        }
    }

    fn palette_confirm(&mut self) -> Task<Message> {
        let Some(result) = self.palette.results.get(self.palette.selected_index) else {
            return Task::none();
        };
        if !result.available {
            return Task::none();
        }
        let id = result.id;
        let input_mode = result.input_mode;

        match input_mode {
            ratatoskr_command_palette::InputMode::Direct => {
                self.palette.close();
                self.update(Message::ExecuteCommand(id))
            }
            ratatoskr_command_palette::InputMode::Parameterized { schema } => {
                // Get the param label for the placeholder text
                let param_label = schema
                    .param_at(0)
                    .map(|p| match p {
                        ratatoskr_command_palette::ParamDef::ListPicker { label } => label,
                        ratatoskr_command_palette::ParamDef::DateTime { label } => label,
                        ratatoskr_command_palette::ParamDef::Enum { label, .. } => label,
                        ratatoskr_command_palette::ParamDef::Text { label, .. } => label,
                    })
                    .unwrap_or("option");

                // Skip DateTime commands for now (snooze picker is complex)
                if matches!(
                    schema.param_at(0),
                    Some(ratatoskr_command_palette::ParamDef::DateTime { .. })
                ) {
                    self.palette.close();
                    return Task::none();
                }

                // Transition to stage 2
                self.palette.stage = PaletteStage::OptionPick;
                self.palette.query.clear();
                self.palette.selected_index = 0;
                self.palette.stage2_command_id = Some(id);
                self.palette.stage2_label = param_label.to_string();
                self.palette.option_items.clear();
                self.palette.option_matches.clear();
                self.palette.options_loading = true;
                self.palette.option_load_generation += 1;
                let generation = self.palette.option_load_generation;

                // Dispatch async resolver call
                let resolver = Arc::clone(&self.resolver);
                let ctx = command_dispatch::build_context(self);
                let focus_task = iced::widget::operation::focus::<Message>(
                    "palette-input".to_string(),
                );
                let load_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            resolver.get_options(id, 0, &[], &ctx)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("spawn_blocking: {e}")))
                    },
                    move |result| {
                        Message::Palette(PaletteMessage::OptionsLoaded(
                            generation, id, result,
                        ))
                    },
                );
                Task::batch([focus_task, load_task])
            }
        }
    }

    fn handle_options_loaded(
        &mut self,
        generation: u64,
        command_id: CommandId,
        result: Result<Vec<ratatoskr_command_palette::OptionItem>, String>,
    ) -> Task<Message> {
        // Discard stale results
        if generation < self.palette.option_load_generation {
            return Task::none();
        }
        // Verify we're still in the right stage for this command
        if self.palette.stage2_command_id != Some(command_id) {
            return Task::none();
        }

        self.palette.options_loading = false;

        match result {
            Ok(items) => {
                self.palette.option_matches =
                    ratatoskr_command_palette::search_options(&items, &self.palette.query);
                self.palette.option_items = items;
                self.palette.selected_index = 0;
            }
            Err(msg) => {
                self.palette.option_items.clear();
                self.palette.option_matches.clear();
                self.status = format!("Palette error: {msg}");
            }
        }
        Task::none()
    }

    fn palette_confirm_option(&mut self) -> Task<Message> {
        let Some(option_match) = self
            .palette
            .option_matches
            .get(self.palette.selected_index)
        else {
            return Task::none();
        };
        if option_match.item.disabled {
            return Task::none();
        }

        let Some(command_id) = self.palette.stage2_command_id else {
            return Task::none();
        };

        let Some(args) = super::commands::build_command_args(command_id, &option_match.item) else {
            return Task::none();
        };

        self.palette.close();
        self.update(Message::ExecuteParameterized(command_id, args))
    }
}
