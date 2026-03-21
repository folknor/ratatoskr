//! Contact autocomplete handler methods for the App.
//!
//! These are extracted handler methods called from the main `update()`
//! dispatch. Each method is one-line dispatched from a `Message` variant
//! in `main.rs`.

use std::sync::Arc;

use iced::Task;

use crate::db::Db;
use crate::pop_out::compose::{ComposeMessage, ComposeState};
use crate::pop_out::PopOutMessage;
use crate::Message;

/// Dispatch an autocomplete search for the active compose field.
///
/// Called when a `TokenInputMessage::TextChanged` is received in a
/// compose window. Increments the generation counter, spawns an async
/// search, and routes the result back to the compose window.
pub fn dispatch_autocomplete_search(
    db: &Arc<Db>,
    window_id: iced::window::Id,
    state: &mut ComposeState,
) -> Task<Message> {
    let query = state.autocomplete.query.clone();
    if query.trim().is_empty() {
        state.autocomplete.results.clear();
        state.autocomplete.highlighted = None;
        return Task::none();
    }

    state.autocomplete.search_generation += 1;
    let generation = state.autocomplete.search_generation;
    let db = Arc::clone(db);

    Task::perform(
        async move { db.search_autocomplete(query, 10).await },
        move |results| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(
                    ComposeMessage::AutocompleteResults(generation, results),
                ),
            )
        },
    )
}

/// Check if a compose message should trigger an autocomplete search.
///
/// Returns true for `TextChanged` messages that indicate the user is
/// typing in a recipient field.
pub fn should_trigger_autocomplete(msg: &ComposeMessage) -> bool {
    matches!(
        msg,
        ComposeMessage::ToTokenInput(
            crate::ui::token_input::TokenInputMessage::TextChanged(_)
        ) | ComposeMessage::CcTokenInput(
            crate::ui::token_input::TokenInputMessage::TextChanged(_)
        ) | ComposeMessage::BccTokenInput(
            crate::ui::token_input::TokenInputMessage::TextChanged(_)
        )
    )
}
