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
        let Some(ctx) = self.action_ctx() else {
            return Task::none();
        };
        let db = Arc::clone(&self.db);
        let filter = self.settings.contact_filter.clone();

        let input = ratatoskr_core::actions::contacts::ContactSaveInput {
            id: entry.id,
            email: entry.email,
            display_name: entry.display_name,
            email2: entry.email2,
            phone: entry.phone,
            company: entry.company,
            notes: entry.notes,
            account_id: entry.account_id,
            source: entry.source,
            server_id: entry.server_id,
        };

        Task::perform(
            async move {
                let outcome =
                    ratatoskr_core::actions::contacts::save_contact(&ctx, input).await;
                // Reload contacts regardless of outcome (local save succeeded
                // for Success and LocalOnly, failed for Failed)
                let contacts = db.get_contacts_for_settings(filter).await;
                (outcome, contacts)
            },
            |(outcome, contacts)| {
                use ratatoskr_core::actions::ActionOutcome;
                match outcome {
                    ActionOutcome::Failed { error } => {
                        log::error!("Contact save failed: {error}");
                        Message::Settings(SettingsMessage::ContactSaved(Err(error.user_message())))
                    }
                    ActionOutcome::LocalOnly { reason, .. } => {
                        // Local save succeeded — reload list so the contact appears.
                        // The degraded state (provider not notified) is logged.
                        // When Settings UI gains a status area, surface reason.user_message().
                        log::warn!("Contact save local-only: {reason}");
                        Message::Settings(SettingsMessage::ContactsLoaded(contacts))
                    }
                    ActionOutcome::Success => {
                        Message::Settings(SettingsMessage::ContactsLoaded(contacts))
                    }
                }
            },
        )
    }

    pub(crate) fn handle_delete_contact(&self, id: String) -> Task<Message> {
        let Some(ctx) = self.action_ctx() else {
            return Task::none();
        };
        let db = Arc::clone(&self.db);
        let filter = self.settings.contact_filter.clone();

        Task::perform(
            async move {
                let outcome =
                    ratatoskr_core::actions::contacts::delete_contact(&ctx, &id).await;
                match outcome {
                    ratatoskr_core::actions::ActionOutcome::Failed { .. } => {
                        // Provider-first delete failed (e.g. JMAP) — contact not
                        // deleted locally. Don't reload (nothing changed).
                        (outcome, None)
                    }
                    _ => {
                        // Success or LocalOnly — contact deleted locally, reload list
                        let contacts = db.get_contacts_for_settings(filter).await.ok();
                        (outcome, contacts)
                    }
                }
            },
            |(outcome, contacts)| {
                use ratatoskr_core::actions::ActionOutcome;
                match outcome {
                    ActionOutcome::Failed { error } => {
                        log::error!("Contact delete failed: {error}");
                        Message::Settings(SettingsMessage::ContactDeleted(Err(error.user_message())))
                    }
                    ActionOutcome::LocalOnly { reason, .. } => {
                        // Local delete succeeded — reload list so the contact disappears.
                        // The degraded state (provider not notified) is logged.
                        // When Settings UI gains a status area, surface reason.user_message().
                        log::warn!("Contact delete local-only: {reason}");
                        if let Some(list) = contacts {
                            Message::Settings(SettingsMessage::ContactsLoaded(Ok(list)))
                        } else {
                            Message::Settings(SettingsMessage::ContactDeleted(Ok(())))
                        }
                    }
                    ActionOutcome::Success => {
                        if let Some(list) = contacts {
                            Message::Settings(SettingsMessage::ContactsLoaded(Ok(list)))
                        } else {
                            Message::Settings(SettingsMessage::ContactDeleted(Ok(())))
                        }
                    }
                }
            },
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

    pub(crate) fn handle_import_contacts(
        &self,
        contacts: Vec<ratatoskr_contact_import::ImportedContact>,
        account_id: Option<String>,
        update_existing: bool,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                execute_contact_import(&db, contacts, account_id, update_existing).await
            },
            |result| {
                let mapped = result.map(|r| crate::ui::settings::ImportResult {
                    imported: r.0,
                    skipped_no_email: r.1,
                    skipped_duplicate: r.2,
                    updated: r.3,
                    groups_created: r.4,
                });
                Message::Settings(SettingsMessage::ImportExecuted(mapped))
            },
        )
    }
}

/// Execute the contact import against the database.
async fn execute_contact_import(
    db: &Arc<Db>,
    contacts: Vec<ratatoskr_contact_import::ImportedContact>,
    account_id: Option<String>,
    update_existing: bool,
) -> Result<(usize, usize, usize, usize, usize), String> {
    let mut imported = 0usize;
    let mut skipped_no_email = 0usize;
    let mut skipped_duplicate = 0usize;
    let mut updated = 0usize;

    for contact in &contacts {
        let Some(email) = contact.normalized_email() else {
            skipped_no_email += 1;
            continue;
        };

        if !email.contains('@') {
            skipped_no_email += 1;
            continue;
        }

        // Check for existing contact by email
        let db_check = Arc::clone(db);
        let email_check = email.clone();
        let exists = db_check
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM contacts WHERE email = ?1 LIMIT 1")
                    .map_err(|e| e.to_string())?;
                let found = stmt
                    .query_row(rusqlite::params![email_check], |row| {
                        row.get::<_, String>(0)
                    })
                    .ok();
                Ok(found)
            })
            .await?;

        if let Some(existing_id) = exists {
            if update_existing {
                let entry = build_contact_entry(
                    existing_id,
                    &email,
                    contact,
                    &account_id,
                );
                db.save_contact(entry).await?;
                updated += 1;
            } else {
                skipped_duplicate += 1;
            }
        } else {
            let entry = build_contact_entry(
                uuid::Uuid::new_v4().to_string(),
                &email,
                contact,
                &account_id,
            );
            db.save_contact(entry).await?;
            imported += 1;
        }
    }

    // Create groups from import and link members
    let mut groups_created = 0usize;
    let mut group_members: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();

    // Collect group memberships
    for contact in &contacts {
        let Some(email) = contact.normalized_email() else {
            continue;
        };
        if !email.contains('@') {
            continue;
        }
        for group_name in &contact.groups {
            let trimmed = group_name.trim();
            if !trimmed.is_empty() {
                group_members
                    .entry(trimmed.to_string())
                    .or_default()
                    .push(email.clone());
            }
        }
    }

    // Create or update each group
    for (group_name, members) in &group_members {
        let db_grp = Arc::clone(db);
        let name = group_name.clone();
        let member_list = members.clone();

        // Check if group already exists by name
        let existing = db_grp
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id FROM contact_groups
                         WHERE name = ?1 LIMIT 1",
                    )
                    .map_err(|e| e.to_string())?;
                let found = stmt
                    .query_row(rusqlite::params![name], |row| {
                        row.get::<_, String>(0)
                    })
                    .ok();
                Ok(found)
            })
            .await?;

        let group_id = existing.unwrap_or_else(|| {
            groups_created += 1;
            uuid::Uuid::new_v4().to_string()
        });

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        #[allow(clippy::cast_possible_wrap)]
        let entry = crate::db::GroupEntry {
            id: group_id,
            name: group_name.clone(),
            member_count: member_list.len() as i64,
            created_at: now as i64,
            updated_at: now as i64,
        };

        db.save_group(entry, member_list).await?;
    }

    Ok((imported, skipped_no_email, skipped_duplicate, updated, groups_created))
}

fn build_contact_entry(
    id: String,
    email: &str,
    contact: &ratatoskr_contact_import::ImportedContact,
    account_id: &Option<String>,
) -> ContactEntry {
    let display_name = contact.effective_display_name();
    let email2 = contact
        .email2
        .as_ref()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty());

    ContactEntry {
        id,
        email: email.to_string(),
        display_name,
        email2,
        phone: contact.phone.clone(),
        company: contact.company.clone(),
        notes: contact.notes.clone(),
        account_id: account_id.clone(),
        account_color: None,
        groups: contact.groups.clone(),
        source: None,
        server_id: None,
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
