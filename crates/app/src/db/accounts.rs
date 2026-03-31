use rusqlite::params;

use super::connection::Db;
use super::types::*;
use crate::ui::thread_list::TypeaheadItem;

impl Db {
    pub async fn get_accounts(&self) -> Result<Vec<Account>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, provider,
                            account_name, account_color, last_sync_at,
                            token_expires_at, is_active,
                            COALESCE(sort_order, 0) AS sort_order
                     FROM accounts
                     ORDER BY sort_order ASC, created_at ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(Account {
                    id: row.get("id")?,
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                    provider: row.get("provider")?,
                    account_name: row.get("account_name")?,
                    account_color: row.get("account_color")?,
                    last_sync_at: row.get("last_sync_at")?,
                    token_expires_at: row.get("token_expires_at")?,
                    is_active: row.get::<_, i64>("is_active")? != 0,
                    sort_order: row.get("sort_order")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load all shared mailboxes for sidebar display, across all accounts.
    pub async fn get_shared_mailboxes(&self) -> Result<Vec<SharedMailbox>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT s.mailbox_id, s.display_name, s.account_id,
                            s.is_sync_enabled, s.last_synced_at, s.sync_error
                     FROM shared_mailbox_sync_state s
                     JOIN accounts a ON s.account_id = a.id
                     WHERE a.is_active = 1
                     ORDER BY a.sort_order ASC, s.display_name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(SharedMailbox {
                    mailbox_id: row.get("mailbox_id")?,
                    display_name: row.get("display_name")?,
                    account_id: row.get("account_id")?,
                    is_sync_enabled: row.get::<_, i64>("is_sync_enabled")? != 0,
                    last_synced_at: row.get("last_synced_at")?,
                    sync_error: row.get("sync_error")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load pinned public folders for sidebar display, across all accounts.
    pub async fn get_pinned_public_folders(&self) -> Result<Vec<PinnedPublicFolder>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT p.folder_id, p.display_name, p.account_id,
                            p.sync_enabled, p.position,
                            COALESCE(f.unread_count, 0) AS unread_count
                     FROM public_folder_pins p
                     LEFT JOIN public_folders f
                       ON p.folder_id = f.id AND p.account_id = f.account_id
                     JOIN accounts a ON p.account_id = a.id
                     WHERE a.is_active = 1
                     ORDER BY p.position ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(PinnedPublicFolder {
                    folder_id: row.get("folder_id")?,
                    display_name: row.get("display_name")?,
                    account_id: row.get("account_id")?,
                    sync_enabled: row.get::<_, i64>("sync_enabled")? != 0,
                    position: row.get("position")?,
                    unread_count: row.get("unread_count")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Persist a discovered shared mailbox (from Autodiscover).
    /// Inserts or updates the shared_mailbox_sync_state row.
    /// Auto-enables sync for newly discovered mailboxes.
    pub async fn upsert_shared_mailbox(
        &self,
        account_id: String,
        mailbox_id: String,
        display_name: Option<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "INSERT INTO shared_mailbox_sync_state
                    (account_id, mailbox_id, display_name, is_sync_enabled)
                 VALUES (?1, ?2, ?3, 1)
                 ON CONFLICT (account_id, mailbox_id)
                 DO UPDATE SET display_name = COALESCE(excluded.display_name, display_name)",
                params![account_id, mailbox_id, display_name],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    pub async fn get_labels(&self, account_id: String) -> Result<Vec<Label>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name FROM labels
                     WHERE account_id = ?1 AND visible = 1
                     ORDER BY sort_order ASC, name ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![account_id], |row| {
                Ok(Label {
                    id: row.get("id")?,
                    name: row.get("name")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Search labels across all accounts for typeahead suggestions.
    ///
    /// Returns up to 10 matches where label name contains the partial
    /// query string (case-insensitive). Includes account email in the
    /// detail field for disambiguation.
    pub async fn search_labels_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            let pattern = format!("%{partial}%");
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT l.name, a.email AS account_email
                     FROM labels l
                     JOIN accounts a ON l.account_id = a.id
                     WHERE l.visible = 1
                       AND l.name LIKE ?1 COLLATE NOCASE
                     ORDER BY l.name ASC
                     LIMIT 10",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pattern], |row| {
                let name: String = row.get("name")?;
                let account_email: String = row.get("account_email")?;
                Ok(TypeaheadItem {
                    label: name.clone(),
                    detail: Some(account_email),
                    insert_value: name,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Search contacts for typeahead suggestions (from:/to: operators).
    pub async fn search_contacts_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            let pattern = format!("%{partial}%");
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT display_name, email
                     FROM seen_addresses
                     WHERE (display_name LIKE ?1 COLLATE NOCASE
                            OR email LIKE ?1 COLLATE NOCASE)
                     ORDER BY last_seen_at DESC
                     LIMIT 10",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pattern], |row| {
                let name: Option<String> = row.get("display_name")?;
                let email: String = row.get("email")?;
                let label = name.as_deref().unwrap_or(&email).to_string();
                Ok(TypeaheadItem {
                    label,
                    detail: Some(email.clone()),
                    insert_value: email,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Search accounts for typeahead suggestions (account: operator).
    pub async fn search_accounts_for_typeahead(
        &self,
        partial: String,
    ) -> Result<Vec<TypeaheadItem>, String> {
        self.with_conn(move |conn| {
            let pattern = format!("%{partial}%");
            let mut stmt = conn
                .prepare(
                    "SELECT id, email, display_name, account_name
                     FROM accounts
                     WHERE (email LIKE ?1 COLLATE NOCASE
                            OR display_name LIKE ?1 COLLATE NOCASE
                            OR account_name LIKE ?1 COLLATE NOCASE)
                     ORDER BY sort_order ASC
                     LIMIT 10",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(params![pattern], |row| {
                let email: String = row.get("email")?;
                let account_name: Option<String> = row.get("account_name")?;
                let display_name: Option<String> = row.get("display_name")?;
                let label = account_name
                    .as_deref()
                    .or(display_name.as_deref())
                    .unwrap_or(&email)
                    .to_string();
                Ok(TypeaheadItem {
                    label: label.clone(),
                    detail: Some(email),
                    insert_value: label,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }
}
