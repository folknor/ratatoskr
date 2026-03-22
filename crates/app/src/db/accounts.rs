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

    pub async fn get_labels(
        &self,
        account_id: String,
    ) -> Result<Vec<Label>, String> {
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
