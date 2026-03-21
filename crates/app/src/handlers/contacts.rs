use std::sync::Arc;

use iced::Task;

use crate::db::{ContactEntry, Db};
use crate::pop_out::compose::{ComposeMessage, ComposeState};
use crate::pop_out::PopOutMessage;
use crate::ui::settings::SettingsMessage;
use crate::{App, Message};

// ── Settings-panel contact/group CRUD ──────────────────

impl App {
    pub(crate) fn handle_load_contacts(&self, filter: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        let group_filter = self.settings.group_filter.clone();
        Task::batch([
            Task::perform(
                async move { db.get_contacts_for_settings(filter).await },
                |result| Message::Settings(SettingsMessage::ContactsLoaded(result)),
            ),
            Task::perform(
                async move { db2.get_groups_for_settings(group_filter).await },
                |result| Message::Settings(SettingsMessage::GroupsLoaded(result)),
            ),
        ])
    }

    pub(crate) fn handle_load_groups(&self, filter: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.get_groups_for_settings(filter).await },
            |result| Message::Settings(SettingsMessage::GroupsLoaded(result)),
        )
    }

    pub(crate) fn handle_save_contact(&self, entry: ContactEntry) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let filter = self.settings.contact_filter.clone();
        Task::perform(
            async move {
                db.save_contact(entry).await?;
                db.get_contacts_for_settings(filter).await
            },
            |result| Message::Settings(SettingsMessage::ContactsLoaded(result)),
        )
    }

    pub(crate) fn handle_delete_contact(&self, id: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let filter = self.settings.contact_filter.clone();
        Task::perform(
            async move {
                db.delete_contact(id).await?;
                db.get_contacts_for_settings(filter).await
            },
            |result| Message::Settings(SettingsMessage::ContactsLoaded(result)),
        )
    }

    pub(crate) fn handle_save_group(
        &self,
        group: crate::db::GroupEntry,
        members: Vec<String>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let filter = self.settings.group_filter.clone();
        Task::perform(
            async move {
                db.save_group(group, members).await?;
                db.get_groups_for_settings(filter).await
            },
            |result| Message::Settings(SettingsMessage::GroupsLoaded(result)),
        )
    }

    pub(crate) fn handle_delete_group(&self, id: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let filter = self.settings.group_filter.clone();
        Task::perform(
            async move {
                db.delete_group(id).await?;
                db.get_groups_for_settings(filter).await
            },
            |result| Message::Settings(SettingsMessage::GroupsLoaded(result)),
        )
    }

    pub(crate) fn handle_load_group_members(&self, group_id: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let gid = group_id.clone();
        Task::perform(
            async move { db.get_group_member_emails(group_id).await },
            move |result| Message::Settings(SettingsMessage::GroupMembersLoaded(gid.clone(), result)),
        )
    }
}

// ── Compose autocomplete ───────────────────────────────

/// Dispatch an autocomplete search for the active compose field.
pub fn dispatch_autocomplete_search(
    db: &Arc<Db>,
    window_id: iced::window::Id,
    state: &mut ComposeState,
) -> Task<Message> {
    let query = state.autocomplete.query.clone();
    log::debug!("Autocomplete search: {query:?}");
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
