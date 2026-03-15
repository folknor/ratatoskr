use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::queries::get_labels;
use crate::db::types::{AccountScope, DbSmartFolder};
use crate::provider::folder_roles::SYSTEM_FOLDER_ROLES;

use crate::db::from_row::FromRow;

use super::scoped_queries::{get_draft_count_with_local, get_unread_counts_by_folder};

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

/// The ordered set of universal sidebar folders.
///
/// Per docs/sidebar/problem-statement.md: Spam and All Mail are omitted from
/// the unified view. Snoozed is included (local feature, works across all
/// providers). Spam only appears when scoped to a specific account.
///
/// Note: SNOOZED is not in `SYSTEM_FOLDER_ROLES` because it has no provider
/// mapping — it's a purely local feature. We define it inline here.
const SIDEBAR_UNIVERSAL_FOLDERS: &[(&str, &str)] = &[
    ("INBOX", "Inbox"),
    ("STARRED", "Starred"),
    ("SNOOZED", "Snoozed"),
    ("SENT", "Sent"),
    ("DRAFT", "Drafts"),
    ("TRASH", "Trash"),
];

/// Universal folders with their unread counts.
///
/// For Drafts, the count includes local-only drafts (from `local_drafts`
/// table) in addition to server-synced draft threads, per the documented
/// requirement in docs/sidebar/problem-statement.md.
fn build_universal_folders(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let counts = get_unread_counts_by_folder(conn, scope)?;
    let draft_count = get_draft_count_with_local(conn, scope)?;

    let folders = SIDEBAR_UNIVERSAL_FOLDERS
        .iter()
        .map(|(id, name)| {
            let unread = if *id == "DRAFT" {
                draft_count
            } else {
                counts
                    .iter()
                    .find(|c| c.folder_id == *id)
                    .map_or(0, |c| c.unread_count)
            };

            NavigationFolder {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                folder_kind: FolderKind::Universal,
                unread_count: unread,
                account_id: None,
            }
        })
        .collect();

    Ok(folders)
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
            // TODO(scaffolding): Smart folder unread counts require executing
            // each folder's query with an is_read=0 filter. Intentionally 0
            // until the smart folder query engine is wired in here.
            unread_count: 0,
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
            stmt.query_map(rusqlite::params![account_id], DbSmartFolder::from_row)
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
            stmt.query_map([], DbSmartFolder::from_row)
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
            // TODO(scaffolding): Per-label unread counts require a query per
            // label (or a batched GROUP BY). Intentionally 0 until we decide
            // whether the cost is acceptable for every navigation refresh.
            unread_count: 0,
            account_id: Some(label.account_id),
        })
        .collect())
}
