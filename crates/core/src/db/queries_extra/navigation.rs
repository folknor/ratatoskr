use std::collections::HashMap;

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

/// Smart folders from the database.
///
/// Smart folders always appear regardless of the current scope — only the
/// sidebar *listing* is unscoped.  Query *execution* (when the user clicks
/// a smart folder) still respects `AccountScope`.
fn build_smart_folders(
    conn: &Connection,
    _scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let smart_folders = query_all_smart_folders_sync(conn)?;

    Ok(smart_folders
        .into_iter()
        .map(|sf| {
            // Smart folders are scope-exempt: always count across all accounts.
            let unread_count = ratatoskr_smart_folder::count_smart_folder_unread(
                conn,
                &sf.query,
                &AccountScope::All,
            )
            .unwrap_or(0);

            NavigationFolder {
                id: sf.id,
                name: sf.name,
                folder_kind: FolderKind::SmartFolder,
                unread_count,
                account_id: sf.account_id,
            }
        })
        .collect())
}

/// Return all smart folders, regardless of scope.
///
/// The old `query_smart_folders_sync` filtered by `AccountScope`, hiding
/// account-specific smart folders in the unified view.  Per the sidebar spec
/// (Phase 1B), smart folders must always be listed.
fn query_all_smart_folders_sync(
    conn: &Connection,
) -> Result<Vec<DbSmartFolder>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM smart_folders ORDER BY sort_order, created_at")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], DbSmartFolder::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Account-specific labels, filtering out system labels.
fn build_account_labels(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<NavigationFolder>, String> {
    let all_labels = get_labels(conn, account_id.to_owned())?;
    let system_ids = system_label_ids();
    let unread_by_label = get_label_unread_counts(conn, account_id)?;

    Ok(all_labels
        .into_iter()
        .filter(|label| !system_ids.contains(&label.id.as_str()))
        .filter(|label| label.visible)
        .map(|label| {
            let unread_count = unread_by_label
                .get(&label.id)
                .copied()
                .unwrap_or(0);

            NavigationFolder {
                id: label.id,
                name: label.name,
                folder_kind: FolderKind::AccountLabel,
                unread_count,
                account_id: Some(label.account_id),
            }
        })
        .collect())
}

/// Batch-fetch unread thread counts for all labels belonging to an account.
///
/// Uses a single GROUP BY query regardless of label count.
fn get_label_unread_counts(
    conn: &Connection,
    account_id: &str,
) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT tl.label_id, COUNT(*) AS unread_count
             FROM threads t
             INNER JOIN thread_labels tl
               ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1 AND t.is_read = 0
             GROUP BY tl.label_id",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut counts = HashMap::new();
    for row in rows {
        let (label_id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(label_id, count);
    }
    Ok(counts)
}
