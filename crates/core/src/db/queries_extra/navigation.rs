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
    /// A provider label/folder specific to one account (container semantics).
    AccountLabel,
    /// A tag-type label — Exchange category, IMAP keyword, JMAP keyword,
    /// or Gmail user label (tag semantics, shown in section 4).
    AccountTag,
}

/// Whether a navigation item is a non-exclusive tag or an exclusive folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LabelSemantics {
    /// Non-exclusive tag (Gmail labels). A message can have multiple.
    Tag,
    /// Exclusive folder (Exchange, IMAP, JMAP). A message lives in exactly one.
    Folder,
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
    /// Parent folder ID for tree rendering. `None` means top-level.
    pub parent_id: Option<String>,
    /// Tag vs Folder semantics. Only meaningful for `AccountLabel` items.
    pub label_semantics: Option<LabelSemantics>,
    /// Query string for smart folders. `None` for regular labels/folders.
    pub query: Option<String>,
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
    log::debug!("Building navigation state for scope={scope:?}");
    let mut folders = build_universal_folders(conn, scope).map_err(|e| {
        log::error!("Failed to build universal folders: {e}");
        e
    })?;
    folders.extend(build_smart_folders(conn, scope).map_err(|e| {
        log::error!("Failed to build smart folders: {e}");
        e
    })?);

    if let AccountScope::Single(account_id) = scope {
        folders.extend(build_account_labels(conn, account_id).map_err(|e| {
            log::error!("Failed to build account labels for {account_id}: {e}");
            e
        })?);
    }

    // Tags (section 4) are always loaded from all accounts, regardless of scope.
    folders.extend(build_all_account_tags(conn).map_err(|e| {
        log::error!("Failed to build account tags: {e}");
        e
    })?);

    log::debug!("Navigation state built: {} folders", folders.len());
    Ok(NavigationState {
        scope: scope.clone(),
        folders,
    })
}

// ── Helpers (each ≤100 lines) ───────────────────────────────

/// The ordered set of universal sidebar folders.
///
/// Per docs/sidebar/problem-statement.md: Spam and All Mail are included here
/// but filtered out in the sidebar UI when in "All Accounts" mode. They appear
/// only when scoped to a single account. Snoozed is included (local feature,
/// works across all providers).
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
    ("SPAM", "Spam"),
    ("ALL_MAIL", "All Mail"),
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
                parent_id: None,
                label_semantics: None,
                query: None,
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
                parent_id: None,
                label_semantics: None,
                query: Some(sf.query),
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
    let provider = get_account_provider(conn, account_id)?;
    let semantics = label_semantics_for_provider(&provider);
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

            // If parent is a system folder (INBOX, SENT, etc.), treat as
            // root — system folders are rendered in the universal section,
            // not in the label tree. Without this, children of system
            // folders become orphans in the tree and get promoted to
            // depth-0 by the orphan recovery path.
            let parent_id = label.parent_label_id.filter(|pid| {
                !system_ids.contains(&pid.as_str())
            });

            let kind = if label.label_kind == "tag" {
                FolderKind::AccountTag
            } else {
                FolderKind::AccountLabel
            };

            NavigationFolder {
                id: label.id,
                name: label.name,
                folder_kind: kind,
                unread_count,
                account_id: Some(label.account_id),
                parent_id,
                label_semantics: Some(semantics.clone()),
                query: None,
            }
        })
        .collect())
}

/// Load all tag-type labels from all accounts, grouped by normalized name.
///
/// Returns one NavigationFolder per unique normalized label name, with
/// an aggregated unread count across all accounts that have that label.
fn build_all_account_tags(
    conn: &Connection,
) -> Result<Vec<NavigationFolder>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.name,
                    COALESCE(SUM(CASE WHEN t.is_read = 0 THEN 1 ELSE 0 END), 0) AS unread_count
             FROM labels l
             LEFT JOIN thread_labels tl ON l.id = tl.label_id AND l.account_id = tl.account_id
             LEFT JOIN threads t ON tl.thread_id = t.id AND tl.account_id = t.account_id
             WHERE l.label_kind = 'tag'
               AND l.visible = 1
             GROUP BY LOWER(TRIM(l.name))
             ORDER BY l.name COLLATE NOCASE ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        let name: String = row.get("name")?;
        let unread_count: i64 = row.get("unread_count")?;
        Ok(NavigationFolder {
            id: format!("tag:{}", name.to_lowercase().trim()),
            name,
            folder_kind: FolderKind::AccountTag,
            unread_count,
            account_id: None, // cross-account
            parent_id: None,
            label_semantics: Some(LabelSemantics::Tag),
            query: None,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Determine label semantics based on the email provider.
fn label_semantics_for_provider(provider: &str) -> LabelSemantics {
    match provider {
        "gmail_api" => LabelSemantics::Tag,
        _ => LabelSemantics::Folder,
    }
}

/// Look up the provider string for an account.
fn get_account_provider(
    conn: &Connection,
    account_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| format!("get_account_provider: {e}"))
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
