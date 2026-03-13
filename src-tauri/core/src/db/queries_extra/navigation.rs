use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::queries::get_labels;
use crate::db::types::{AccountScope, DbSmartFolder};
use crate::provider::folder_roles::SYSTEM_FOLDER_ROLES;

use super::scoped_queries::get_unread_counts_by_folder;
use super::row_to_smart_folder;

// ── Types ───────────────────────────────────────────────────

/// The kind of folder shown in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FolderKind {
    /// A well-known system folder (Inbox, Sent, Drafts, etc.).
    Universal,
    /// A user-defined smart folder backed by a saved query.
    SmartFolder,
    /// A provider label/folder specific to one account.
    AccountLabel,
}

/// A single item in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationFolder {
    pub id: String,
    pub name: String,
    pub folder_kind: FolderKind,
    pub unread_count: i64,
    pub account_id: Option<String>,
}

/// The complete navigation state returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationState {
    pub scope: AccountScope,
    pub folders: Vec<NavigationFolder>,
}

// ── System label IDs to filter out from account labels ──────

/// Collect all label IDs from `SYSTEM_FOLDER_ROLES` that should be hidden
/// when listing an account's custom labels.
fn system_label_ids() -> Vec<&'static str> {
    SYSTEM_FOLDER_ROLES.iter().map(|r| r.label_id).collect()
}

// ── Public API ──────────────────────────────────────────────

/// Build the full navigation state the sidebar needs in one call.
///
/// Returns universal folders with unread counts, smart folders, and
/// (when scoped to a single account) that account's non-system labels.
pub fn get_navigation_state(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<NavigationState, String> {
    let mut folders = build_universal_folders(conn, scope)?;
    folders.extend(build_smart_folders(conn, scope)?);

    if let AccountScope::Single(account_id) = scope {
        folders.extend(build_account_labels(conn, account_id)?);
    }

    Ok(NavigationState {
        scope: scope.clone(),
        folders,
    })
}

// ── Helpers (each ≤100 lines) ───────────────────────────────

/// Universal folders with their unread counts.
fn build_universal_folders(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let counts = get_unread_counts_by_folder(conn, scope)?;

    let folders = SYSTEM_FOLDER_ROLES
        .iter()
        .filter(|role| is_sidebar_universal_folder(role.label_id))
        .map(|role| {
            let unread = counts
                .iter()
                .find(|c| c.folder_id == role.label_id)
                .map_or(0, |c| c.unread_count);

            NavigationFolder {
                id: role.label_id.to_owned(),
                name: role.label_name.to_owned(),
                folder_kind: FolderKind::Universal,
                unread_count: unread,
                account_id: None,
            }
        })
        .collect();

    Ok(folders)
}

/// Which system folders appear in the sidebar as universal items.
fn is_sidebar_universal_folder(label_id: &str) -> bool {
    matches!(
        label_id,
        "INBOX" | "STARRED" | "SENT" | "DRAFT" | "TRASH" | "SPAM"
    )
}

/// Smart folders from the database, scoped appropriately.
fn build_smart_folders(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let smart_folders = query_smart_folders_sync(conn, scope)?;

    Ok(smart_folders
        .into_iter()
        .map(|sf| NavigationFolder {
            id: sf.id,
            name: sf.name,
            folder_kind: FolderKind::SmartFolder,
            unread_count: 0, // smart folders don't have a traditional unread count
            account_id: sf.account_id,
        })
        .collect())
}

/// Synchronous query for smart folders (the existing helper is async via `DbState`).
fn query_smart_folders_sync(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<DbSmartFolder>, String> {
    match scope {
        AccountScope::Single(account_id) => {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM smart_folders WHERE account_id IS NULL OR account_id = ?1
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(rusqlite::params![account_id], row_to_smart_folder)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
        AccountScope::Multiple(_) | AccountScope::All => {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM smart_folders WHERE account_id IS NULL
                     ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], row_to_smart_folder)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
    }
}

/// Account-specific labels, filtering out system labels.
fn build_account_labels(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<NavigationFolder>, String> {
    let all_labels = get_labels(conn, account_id.to_owned())?;
    let system_ids = system_label_ids();

    Ok(all_labels
        .into_iter()
        .filter(|label| !system_ids.contains(&label.id.as_str()))
        .filter(|label| label.visible)
        .map(|label| NavigationFolder {
            id: label.id,
            name: label.name,
            folder_kind: FolderKind::AccountLabel,
            unread_count: 0, // label-level unread counts require a separate query
            account_id: Some(label.account_id),
        })
        .collect())
}
