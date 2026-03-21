use std::sync::Arc;

use iced::Task;

use crate::db::ContactEntry;
use crate::ui::settings::SettingsMessage;
use crate::{App, Message};

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
