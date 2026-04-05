use rtsk::contacts::search::{ContactSearchKind, search_contacts_unified};
use rtsk::db::queries_extra::{
    ContactSettingsEntry, GroupSettingsEntry, delete_group_sync, load_contacts_for_settings_sync,
    load_group_member_emails_sync, load_groups_for_settings_sync, save_contact_sync,
    save_group_sync,
};

use super::connection::Db;

// ── Contact search types ─────────────────────────────────────

/// A contact result from the autocomplete search.
#[derive(Debug, Clone)]
pub struct ContactMatch {
    pub email: String,
    pub display_name: Option<String>,
    /// Whether this is a group result.
    pub is_group: bool,
    /// Group ID (only set for group results).
    pub group_id: Option<String>,
    /// Member count (only set for group results).
    pub member_count: Option<i64>,
}

// ── Async autocomplete wrapper for Db ─────────────────────────

impl Db {
    /// Async wrapper for autocomplete search, suitable for
    /// `Task::perform`.
    pub async fn search_autocomplete(
        &self,
        query: String,
        limit: i64,
    ) -> Result<Vec<ContactMatch>, String> {
        self.with_conn(move |conn| {
            Ok(search_contacts_unified(conn, &query, limit)?
                .into_iter()
                .map(|row| match row.kind {
                    ContactSearchKind::Contact | ContactSearchKind::SeenAddress => ContactMatch {
                        email: row.email,
                        display_name: row.display_name,
                        is_group: false,
                        group_id: None,
                        member_count: None,
                    },
                    ContactSearchKind::Group {
                        group_id,
                        member_count,
                    } => ContactMatch {
                        email: row.email,
                        display_name: row.display_name,
                        is_group: true,
                        group_id: Some(group_id),
                        member_count: Some(member_count),
                    },
                })
                .collect())
        })
        .await
    }
}

// ── Contact management types ─────────────────────────────────

/// A contact entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct ContactEntry {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub account_color: Option<String>,
    pub groups: Vec<String>,
    /// Contact source: "user", "google", "graph", "carddav".
    /// Used to determine save behavior: local contacts save immediately,
    /// synced contacts use explicit Save with provider write-back.
    pub source: Option<String>,
    /// Provider-assigned server ID for synced contacts. Used by the action
    /// service for write-back dispatch without ambiguous email-based lookups.
    pub server_id: Option<String>,
}

/// A contact group entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Contact management CRUD ──────────────────────────────────

impl Db {
    pub async fn find_contact_id_by_email(&self, email: String) -> Result<Option<String>, String> {
        let db = self.read_db_state();
        rtsk::db::queries_extra::db_find_contact_id_by_email(&db, email).await
    }

    pub async fn find_group_id_by_name(&self, name: String) -> Result<Option<String>, String> {
        let db = self.read_db_state();
        rtsk::db::queries_extra::db_find_contact_group_id_by_name(&db, name).await
    }

    /// Load contacts for the settings management list, optionally
    /// filtered.
    pub async fn get_contacts_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<ContactEntry>, String> {
        self.with_conn(move |conn| {
            Ok(load_contacts_for_settings_sync(conn, &filter)?
                .into_iter()
                .map(|row| ContactEntry {
                    id: row.id,
                    email: row.email,
                    display_name: row.display_name,
                    email2: row.email2,
                    phone: row.phone,
                    company: row.company,
                    notes: row.notes,
                    account_id: row.account_id,
                    account_color: row.account_color,
                    groups: row.groups,
                    source: row.source,
                    server_id: row.server_id,
                })
                .collect())
        })
        .await
    }

    /// Load contact groups for the settings management list.
    pub async fn get_groups_for_settings(&self, filter: String) -> Result<Vec<GroupEntry>, String> {
        self.with_conn(move |conn| {
            Ok(load_groups_for_settings_sync(conn, &filter)?
                .into_iter()
                .map(|row| GroupEntry {
                    id: row.id,
                    name: row.name,
                    member_count: row.member_count,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
                .collect())
        })
        .await
    }

    /// Get member emails for a group.
    pub async fn get_group_member_emails(&self, group_id: String) -> Result<Vec<String>, String> {
        self.with_conn(move |conn| load_group_member_emails_sync(conn, &group_id))
            .await
    }

    /// Expand a contact group into individual (email, display_name) pairs.
    /// Recursively expands nested groups with cycle detection.
    pub async fn expand_contact_group(
        &self,
        group_id: String,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let db = self.read_db_state();
        Ok(rtsk::db::queries_extra::db_expand_contact_group_with_names(&db, group_id)
            .await?
            .into_iter()
            .map(|row| (row.email, row.display_name))
            .collect())
    }

    /// Insert or update a contact.
    pub async fn save_contact(&self, entry: ContactEntry) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_contact_sync(
                conn,
                &ContactSettingsEntry {
                    id: entry.id,
                    email: entry.email,
                    display_name: entry.display_name,
                    email2: entry.email2,
                    phone: entry.phone,
                    company: entry.company,
                    notes: entry.notes,
                    account_id: entry.account_id,
                    account_color: entry.account_color,
                    groups: entry.groups,
                    source: entry.source,
                    server_id: entry.server_id,
                },
            )
        })
        .await
    }

    /// Insert or update a contact group.
    pub async fn save_group(
        &self,
        group: GroupEntry,
        member_emails: Vec<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_group_sync(
                conn,
                &GroupSettingsEntry {
                    id: group.id,
                    name: group.name,
                    member_count: group.member_count,
                    created_at: group.created_at,
                    updated_at: group.updated_at,
                },
                &member_emails,
            )
        })
        .await
    }

    /// Delete a contact group by ID.
    pub async fn delete_group(&self, group_id: String) -> Result<(), String> {
        self.with_write_conn(move |conn| delete_group_sync(conn, &group_id))
        .await
    }
}
