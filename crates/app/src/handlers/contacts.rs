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
        let source = entry.source.clone();
        let email = entry.email.clone();
        let phone = entry.phone.clone();
        let company = entry.company.clone();
        let notes = entry.notes.clone();

        // Save locally first, then attempt provider write-back for synced contacts
        Task::perform(
            async move {
                db.save_contact(entry).await?;

                // Provider write-back (best-effort — errors logged, not surfaced)
                if let Some(ref src) = source {
                    let wb_result = dispatch_provider_write_back(
                        &db, src, &email, phone.as_deref(),
                        company.as_deref(), notes.as_deref(),
                    )
                    .await;
                    if let Err(e) = wb_result {
                        log::warn!("Contact write-back failed for {email}: {e}");
                    }
                }

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
    }
}

// ── Provider write-back ─────────────────────────────────

/// Dispatch a contact write-back to the appropriate provider.
///
/// This is called after the local save completes. It's best-effort —
/// errors are returned for logging but do not prevent the local save.
/// Display name changes are NOT pushed (local-only override per spec).
async fn dispatch_provider_write_back(
    db: &Arc<Db>,
    source: &str,
    email: &str,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> Result<(), String> {
    match source {
        "google" => {
            // Google People API write-back: scaffolding exists in
            // core::contacts::sync_google but HTTP push not yet wired.
            // The body builder and server info lookup are ready — once
            // GmailClient is available at this layer, wire:
            //   1. get_google_contact_server_info(db, email)
            //   2. build_google_contact_update_body(phone, company, etag)
            //   3. client.patch_absolute(url, body)
            log::info!("Google contact write-back queued for {email} (not yet wired to HTTP)");
            Ok(())
        }
        "graph" => {
            // Microsoft Graph write-back: scaffolding exists in
            // core::contacts::sync_graph but HTTP push not yet wired.
            //   1. get_graph_contact_server_info(db, email)
            //   2. build_graph_contact_update_body(phone, company)
            //   3. client.patch(url, body)
            log::info!("Graph contact write-back queued for {email} (not yet wired to HTTP)");
            Ok(())
        }
        "jmap" => {
            // JMAP ContactCard/set is fully implemented in
            // crates/jmap/src/contacts_sync.rs::jmap_contacts_push_update.
            // Need JmapClient instance to call it. When sync orchestrator
            // provides client access, wire:
            //   1. get_jmap_contact_server_info(db, email)
            //   2. jmap_contacts_push_update(client, server_id, phone, company, notes)
            log::info!("JMAP contact write-back queued for {email} (not yet wired to client)");
            Ok(())
        }
        "carddav" => {
            // CardDAV PUT is not yet implemented in the CardDavClient.
            // Needs: vCard builder + PUT method on client.
            log::info!("CardDAV contact write-back not yet supported for {email}");
            Ok(())
        }
        "user" => Ok(()), // Local contacts don't need write-back
        other => {
            log::warn!("Unknown contact source '{other}' for write-back");
            Ok(())
        }
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
