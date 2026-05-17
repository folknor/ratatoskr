use std::collections::HashMap;

use crate::db::{Connection, OptionalExtension, ToSql, params};
use serde::{Deserialize, Serialize};

use crate::db::queries::get_folders;
use crate::db::types::{AccountScope, DbFolder, DbSmartFolder};
use crate::provider::folder_roles::SYSTEM_FOLDER_ROLES;

use crate::db::from_row::FromRow;

use crate::db::queries_extra::scoped_queries::get_unread_counts_by_folder;
use crate::db::types::UniversalUnreadCount;

// ── Types ───────────────────────────────────────────────────

/// The kind of folder shown in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FolderKind {
    /// A well-known system folder (Inbox, Sent, Drafts, etc.).
    Universal,
    /// A user-defined smart folder backed by a saved query.
    SmartFolder,
    /// A user-created provider folder specific to one account.
    AccountFolder,
    /// A user-created cross-account label group.
    LabelGroup,
}

/// Mailbox rights for permission gating in the UI.
///
/// All fields default to `None` (= unknown / not applicable).
/// `Some(true)` = permitted, `Some(false)` = denied.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxRightsInfo {
    pub may_read_items: Option<bool>,
    pub may_add_items: Option<bool>,
    pub may_remove_items: Option<bool>,
    pub may_set_seen: Option<bool>,
    pub may_set_keywords: Option<bool>,
    pub may_create_child: Option<bool>,
    pub may_rename: Option<bool>,
    pub may_delete: Option<bool>,
    pub may_submit: Option<bool>,
}

impl MailboxRightsInfo {
    /// Returns `true` if any right is explicitly set (not all `None`).
    pub fn is_known(&self) -> bool {
        self.may_read_items.is_some()
    }
}

/// A single item in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationFolder {
    pub id: String,
    pub name: String,
    pub folder_kind: FolderKind,
    unread_count: NavigationUnreadCount,
    pub account_id: Option<String>,
    /// Parent folder ID for tree rendering. `None` means top-level.
    pub parent_id: Option<String>,
    /// Query string for smart folders. `None` for regular labels/folders.
    pub query: Option<String>,
    /// Mailbox rights from JMAP/IMAP ACL. `None` for non-shared or unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rights: Option<MailboxRightsInfo>,
    /// JMAP subscription state. `None` for non-JMAP providers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_subscribed: Option<bool>,
    /// Resolved background color for the label-group dot/chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_bg: Option<String>,
    /// Resolved foreground color, paired with `color_bg`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_fg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NavigationUnreadCount {
    Universal(UniversalUnreadCount),
    General(i64),
}

impl NavigationUnreadCount {
    pub fn as_i64(&self) -> i64 {
        match self {
            Self::Universal(count) => count.as_i64(),
            Self::General(count) => *count,
        }
    }

    pub fn as_universal(&self) -> Option<UniversalUnreadCount> {
        match self {
            Self::Universal(count) => Some(*count),
            Self::General(_) => None,
        }
    }
}

impl NavigationFolder {
    pub fn unread_count(&self) -> i64 {
        self.unread_count.as_i64()
    }

    pub fn universal_unread_count(&self) -> Option<UniversalUnreadCount> {
        self.unread_count.as_universal()
    }
}

/// The complete navigation state returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationState {
    pub scope: AccountScope,
    pub folders: Vec<NavigationFolder>,
}

// ── System folder IDs to filter out from account folders ────

/// Collect all folder IDs from `SYSTEM_FOLDER_ROLES` that should be hidden
/// when listing an account's custom folders.
fn system_folder_ids() -> Vec<&'static str> {
    SYSTEM_FOLDER_ROLES.iter().map(|r| r.label_id).collect()
}

// ── Public API ──────────────────────────────────────────────

/// Build the full navigation state the sidebar needs in one call.
///
/// Returns universal folders with unread counts, smart folders, and
/// (when scoped to a single account) that account's non-system labels.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
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
    folders.extend(build_label_groups(conn, scope).map_err(|e| {
        log::error!("Failed to build label groups: {e}");
        e
    })?);

    if let AccountScope::Single(account_id) = scope {
        folders.extend(build_account_folders(conn, account_id).map_err(|e| {
            log::error!("Failed to build account folders for {account_id}: {e}");
            e
        })?);
    }

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
/// mapping - it's a purely local feature. We define it inline here.
const SIDEBAR_UNIVERSAL_FOLDERS: &[(&str, &str)] = &[
    ("INBOX", "Inbox"),
    ("STARRED", "Starred"),
    ("SNOOZED", "Snoozed"),
    ("SENT", "Sent"),
    ("DRAFT", "Drafts"),
    ("archive", "Archive"),
    ("TRASH", "Trash"),
    ("SPAM", "Spam"),
    ("all-mail", "All Mail"),
];

/// Universal folders with their unread counts.
///
/// Every universal pill - Drafts included - counts the `is_read = 0`
/// subset of the folder's synced thread membership. Rationale and the
/// local-drafts carve-out: `reference/glossary/drafts.md` § "Count semantics."
fn build_universal_folders(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let counts = get_unread_counts_by_folder(conn, scope)?;

    let folders = SIDEBAR_UNIVERSAL_FOLDERS
        .iter()
        .map(|(id, name)| {
            let unread = counts
                .iter()
                .find(|c| c.folder_id == *id)
                .map_or_else(UniversalUnreadCount::default, |c| c.unread_count);

            NavigationFolder {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                folder_kind: FolderKind::Universal,
                unread_count: NavigationUnreadCount::Universal(unread),
                account_id: None,
                parent_id: None,
                query: None,
                rights: None,
                is_subscribed: None,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect();

    Ok(folders)
}

/// Smart folders from the database.
///
/// Smart folders always appear regardless of the current scope - only the
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
            let unread_count =
                smart_folder::count_smart_folder_unread(conn, &sf.query, &AccountScope::All)
                    .unwrap_or(0);

            NavigationFolder {
                id: sf.id,
                name: sf.name,
                folder_kind: FolderKind::SmartFolder,
                unread_count: NavigationUnreadCount::General(unread_count),
                account_id: sf.account_id,
                parent_id: None,
                query: Some(sf.query),
                rights: None,
                is_subscribed: None,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect())
}

/// Return all smart folders, regardless of scope.
///
/// The old `query_smart_folders_sync` filtered by `AccountScope`, hiding
/// account-specific smart folders in the unified view.  Per the sidebar spec
/// (Phase 1B), smart folders must always be listed.
fn query_all_smart_folders_sync(conn: &Connection) -> Result<Vec<DbSmartFolder>, String> {
    let mut stmt = conn
        .prepare("SELECT * FROM smart_folders ORDER BY sort_order, created_at")
        .map_err(|e| e.to_string())?;
    stmt.query_map([], DbSmartFolder::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Account-specific folders, filtering out system folders.
fn build_account_folders(
    conn: &Connection,
    account_id: &str,
) -> Result<Vec<NavigationFolder>, String> {
    let all_folders = get_folders(conn, account_id)?;
    let system_ids = system_folder_ids();
    let unread_by_folder = get_folder_unread_counts(conn, account_id)?;

    Ok(all_folders
        .into_iter()
        .filter(|folder| !system_ids.contains(&folder.id.as_str()))
        .filter(|folder| folder.visible)
        .map(|folder| {
            let unread_count = unread_by_folder.get(&folder.id).copied().unwrap_or(0);

            let rights = rights_from_folder(&folder);

            // If parent is a system folder (INBOX, SENT, etc.), treat as
            // root - system folders are rendered in the universal section,
            // not in the folder tree. Without this, children of system
            // folders become orphans in the tree and get promoted to
            // depth-0 by the orphan recovery path.
            let parent_id = folder
                .parent_id
                .filter(|pid| !system_ids.contains(&pid.as_str()));

            NavigationFolder {
                is_subscribed: folder.is_subscribed,
                id: folder.id,
                name: folder.name,
                folder_kind: FolderKind::AccountFolder,
                unread_count: NavigationUnreadCount::General(unread_count),
                account_id: Some(folder.account_id),
                parent_id,
                query: None,
                rights,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect())
}

/// Explicit label groups for the sidebar LABELS section.
fn build_label_groups(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<Vec<NavigationFolder>, String> {
    let unread_by_group = load_label_group_unread_counts(conn, scope)?;
    build_label_groups_from_counts(conn, &unread_by_group)
}

fn build_label_groups_from_counts(
    conn: &Connection,
    unread_by_group: &HashMap<i64, i64>,
) -> Result<Vec<NavigationFolder>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, color_bg, color_fg
             FROM label_groups
             ORDER BY name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>("id")?,
                row.get::<_, String>("name")?,
                row.get::<_, String>("color_bg")?,
                row.get::<_, String>("color_fg")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut groups = Vec::new();
    for row in rows {
        let (id, name, color_bg, color_fg) = row.map_err(|e| e.to_string())?;
        groups.push(NavigationFolder {
            is_subscribed: None,
            id: id.to_string(),
            name,
            folder_kind: FolderKind::LabelGroup,
            unread_count: NavigationUnreadCount::General(
                unread_by_group.get(&id).copied().unwrap_or(0),
            ),
            account_id: None,
            parent_id: None,
            query: None,
            rights: None,
            color_bg: Some(color_bg),
            color_fg: Some(color_fg),
        });
    }
    Ok(groups)
}

/// Batch-fetch unread thread counts for all folders belonging to an account.
fn get_folder_unread_counts(
    conn: &Connection,
    account_id: &str,
) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT tf.folder_id, COUNT(*) AS unread_count
             FROM threads t
             INNER JOIN thread_folders tf
               ON tf.account_id = t.account_id AND tf.thread_id = t.id
             WHERE t.account_id = ?1 AND t.is_read = 0
               AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
             GROUP BY tf.folder_id",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut counts = HashMap::new();
    for row in rows {
        let (folder_id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(folder_id, count);
    }
    Ok(counts)
}

fn scope_clause_for_threads(
    scope: &AccountScope,
    base_idx: usize,
) -> (String, Vec<Box<dyn ToSql>>) {
    match scope {
        AccountScope::Single(id) => (
            format!("t.account_id = ?{base_idx}"),
            vec![Box::new(id.clone())],
        ),
        AccountScope::Multiple(ids) => {
            if ids.is_empty() {
                return ("0=1".to_owned(), Vec::new());
            }
            let placeholders: Vec<String> = ids
                .iter()
                .enumerate()
                .map(|(idx, _)| format!("?{}", base_idx + idx))
                .collect();
            let params: Vec<Box<dyn ToSql>> =
                ids.iter().map(|id| Box::new(id.clone()) as _).collect();
            (format!("t.account_id IN ({})", placeholders.join(", ")), params)
        }
        AccountScope::All => ("1=1".to_owned(), Vec::new()),
    }
}

fn load_label_group_unread_counts(
    conn: &Connection,
    scope: &AccountScope,
) -> Result<HashMap<i64, i64>, String> {
    let (scope_clause, scope_params) = scope_clause_for_threads(scope, 1);
    let group_fragment = crate::db::queries_extra::user_visible_label_group_rendered_fragment(
        "t.account_id",
        "t.id",
        "lg.id = lg_outer.id",
    );
    // Cross-join threads × label_groups: the per-pair EXISTS in
    // `group_fragment` is the membership filter, so every (thread, group)
    // pair is tested independently. The inner GROUP BY collapses any
    // duplicate row that the merge algebra would otherwise produce.
    let sql = format!(
        "SELECT group_id, COUNT(*) AS unread_count
         FROM (
           SELECT t.account_id, t.id AS thread_id, lg_outer.id AS group_id
           FROM threads t
           INNER JOIN label_groups lg_outer
           WHERE {scope_clause}
             AND t.is_read = 0
             AND t.shared_mailbox_id IS NULL
             AND t.is_chat_thread = 0
             AND {group_fragment}
           GROUP BY t.account_id, t.id, lg_outer.id
         )
         GROUP BY group_id"
    );
    let params: Vec<&dyn ToSql> =
        scope_params.iter().map(AsRef::as_ref).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params.as_slice(), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut counts = HashMap::new();
    for row in rows {
        let (group_id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(group_id, count);
    }
    Ok(counts)
}

fn load_label_group_unread_counts_for_shared_mailbox(
    conn: &Connection,
    account_id: &str,
    mailbox_id: &str,
) -> Result<HashMap<i64, i64>, String> {
    let group_fragment = crate::db::queries_extra::user_visible_label_group_rendered_fragment(
        "t.account_id",
        "t.id",
        "lg.id = lg_outer.id",
    );
    // See `load_label_group_unread_counts` for the cross-join shape.
    let sql = format!(
        "SELECT group_id, COUNT(*) AS unread_count
         FROM (
           SELECT t.account_id, t.id AS thread_id, lg_outer.id AS group_id
           FROM threads t
           INNER JOIN label_groups lg_outer
           WHERE t.account_id = ?1
             AND t.shared_mailbox_id = ?2
             AND t.is_read = 0
             AND t.is_chat_thread = 0
             AND {group_fragment}
           GROUP BY t.account_id, t.id, lg_outer.id
         )
         GROUP BY group_id"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![account_id, mailbox_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;

    let mut counts = HashMap::new();
    for row in rows {
        let (group_id, count) = row.map_err(|e| e.to_string())?;
        counts.insert(group_id, count);
    }
    Ok(counts)
}

/// Navigation state for a shared mailbox scope.
///
/// Returns the shared mailbox's folder list with unread counts scoped to
/// threads belonging to that mailbox.
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn get_shared_mailbox_navigation(
    conn: &Connection,
    account_id: &str,
    mailbox_id: &str,
) -> Result<NavigationState, String> {
    let all_folders = get_folders(conn, account_id)?;
    let system_ids = system_folder_ids();

    // Unread counts for folders, scoped to this shared mailbox.
    let mut folder_unread_stmt = conn
        .prepare(
            "SELECT tf.folder_id, COUNT(*) AS unread_count
             FROM threads t
             INNER JOIN thread_folders tf
               ON tf.account_id = t.account_id AND tf.thread_id = t.id
             WHERE t.account_id = ?1 AND t.shared_mailbox_id = ?2
               AND t.is_read = 0
             GROUP BY tf.folder_id",
        )
        .map_err(|e| e.to_string())?;
    let unread_by_folder: HashMap<String, i64> = folder_unread_stmt
        .query_map(params![account_id, mailbox_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    let unread_by_label_group =
        load_label_group_unread_counts_for_shared_mailbox(conn, account_id, mailbox_id)?;

    // Universal folders with shared-mailbox-scoped unread counts
    let mut folders: Vec<NavigationFolder> = SIDEBAR_UNIVERSAL_FOLDERS
        .iter()
        .map(|(id, name)| {
            let unread = unread_by_folder.get(*id).copied().unwrap_or(0);
            NavigationFolder {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                folder_kind: FolderKind::Universal,
                unread_count: NavigationUnreadCount::Universal(UniversalUnreadCount::from_synced_thread_count(unread)),
                account_id: None,
                parent_id: None,
                query: None,
                rights: None,
                is_subscribed: None,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect();

    // Account folders (non-system, visible) with shared-mailbox-scoped counts.
    let account_folders: Vec<NavigationFolder> = all_folders
        .into_iter()
        .filter(|f| !system_ids.contains(&f.id.as_str()) && f.visible)
        .map(|folder| {
            let unread = unread_by_folder.get(&folder.id).copied().unwrap_or(0);
            let rights = rights_from_folder(&folder);
            let parent_id = folder
                .parent_id
                .filter(|pid| !system_ids.contains(&pid.as_str()));
            NavigationFolder {
                is_subscribed: folder.is_subscribed,
                id: folder.id,
                name: folder.name,
                folder_kind: FolderKind::AccountFolder,
                unread_count: NavigationUnreadCount::General(unread),
                account_id: Some(folder.account_id),
                parent_id,
                query: None,
                rights,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect();
    folders.extend(account_folders);

    folders.extend(build_label_groups_from_counts(
        conn,
        &unread_by_label_group,
    )?);

    Ok(NavigationState {
        scope: AccountScope::Single(account_id.to_string()),
        folders,
    })
}

/// Extract mailbox rights from a `DbFolder` into a `MailboxRightsInfo`.
///
/// Returns `None` if no rights are set (all fields are `None`), meaning
/// the provider doesn't supply rights data for this folder.
fn rights_from_folder(folder: &DbFolder) -> Option<MailboxRightsInfo> {
    folder.right_read?;
    Some(MailboxRightsInfo {
        may_read_items: folder.right_read,
        may_add_items: folder.right_add,
        may_remove_items: folder.right_remove,
        may_set_seen: folder.right_set_seen,
        may_set_keywords: folder.right_set_keywords,
        may_create_child: folder.right_create_child,
        may_rename: folder.right_rename,
        may_delete: folder.right_delete,
        may_submit: folder.right_submit,
    })
}

// ── Shared mailbox queries ─────────────────────────────────

/// A shared/delegated mailbox row for sidebar display.
#[derive(Debug, Clone)]
pub struct SharedMailboxRow {
    pub mailbox_id: String,
    pub display_name: Option<String>,
    pub account_id: String,
    pub is_sync_enabled: bool,
    pub last_synced_at: Option<i64>,
    pub sync_error: Option<String>,
}

/// Load all shared mailboxes for sidebar display, across all active accounts.
pub fn get_shared_mailboxes_sync(conn: &Connection) -> Result<Vec<SharedMailboxRow>, String> {
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
        Ok(SharedMailboxRow {
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
}

/// Look up the email address for a shared mailbox.
///
/// Used by pop-out compose to determine the sender identity for shared
/// mailbox contexts - not a sidebar boot query.
pub fn get_shared_mailbox_email_sync(
    conn: &Connection,
    account_id: &str,
    mailbox_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT email_address FROM shared_mailbox_sync_state
         WHERE account_id = ?1 AND mailbox_id = ?2",
        params![account_id, mailbox_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map_err(|e| format!("shared mailbox email: {e}"))
    .map(Option::flatten)
}

// ── Pinned public folder queries ───────────────────────────

/// A pinned public folder row for sidebar display.
#[derive(Debug, Clone)]
pub struct PinnedPublicFolderRow {
    pub folder_id: String,
    pub display_name: String,
    pub account_id: String,
    pub sync_enabled: bool,
    pub position: i64,
    pub unread_count: i64,
}

/// Load pinned public folders for sidebar display, across all active accounts.
pub fn get_pinned_public_folders_sync(
    conn: &Connection,
) -> Result<Vec<PinnedPublicFolderRow>, String> {
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
        Ok(PinnedPublicFolderRow {
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{FolderKind, get_navigation_state};
    use crate::db::migrations;
    use crate::db::types::AccountScope;

    #[test]
    fn drafts_universal_pill_uses_unread_synced_threads_only() {
        let conn = crate::db::Connection::open_in_memory().unwrap();
        migrations::run_all(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.com', 'graph')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders (id, account_id, name) VALUES ('DRAFT', 'acc', 'Drafts')",
            [],
        )
        .unwrap();
        for (thread_id, is_read) in [("read-draft", 1), ("unread-draft", 0)] {
            conn.execute(
                "INSERT INTO threads (id, account_id, subject, snippet, last_message_at, \
                 message_count, is_read) VALUES (?1, 'acc', 'draft', 'draft', 1, 1, ?2)",
                crate::db::params![thread_id, is_read],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO thread_folders (account_id, thread_id, folder_id) \
                 VALUES ('acc', ?1, 'DRAFT')",
                crate::db::params![thread_id],
            )
            .unwrap();
        }
        for id in ["local-1", "local-2", "local-3"] {
            conn.execute(
                "INSERT INTO local_drafts (id, account_id, subject, updated_at, sync_status) \
                 VALUES (?1, 'acc', 'local draft', 1, 'pending')",
                crate::db::params![id],
            )
            .unwrap();
        }

        let nav = get_navigation_state(&conn, &AccountScope::Single("acc".to_string())).unwrap();
        let drafts = nav
            .folders
            .iter()
            .find(|folder| matches!(folder.folder_kind, FolderKind::Universal) && folder.id == "DRAFT")
            .unwrap();

        assert_eq!(drafts.unread_count(), 1);
        assert_eq!(drafts.universal_unread_count().unwrap().as_i64(), 1);
    }
}

// ── Operator typeahead queries ─────────────────────────────
//
// Search-operator typeahead queries for the search bar's `label:`,
// `folder:`, `from:`, `to:`, and `account:` completions.

/// A label row for search-operator typeahead.
#[derive(Debug, Clone)]
pub struct LabelTypeaheadRow {
    pub name: String,
    pub account_email: String,
}

/// Search visible labels for `label:` / `folder:` operator typeahead.
pub fn search_labels_for_typeahead_sync(
    conn: &Connection,
    query: &str,
) -> Result<Vec<LabelTypeaheadRow>, String> {
    let pattern = crate::db::make_like_pattern(query.trim());
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT l.name, a.email AS account_email
             FROM labels l
             JOIN accounts a ON l.account_id = a.id
             WHERE l.visible = 1
               AND l.name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
             ORDER BY l.name ASC
             LIMIT 10",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![pattern], |row| {
        Ok(LabelTypeaheadRow {
            name: row.get("name")?,
            account_email: row.get("account_email")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// A seen-address row for search-operator typeahead.
#[derive(Debug, Clone)]
pub struct SeenAddressTypeaheadRow {
    pub email: String,
    pub display_name: Option<String>,
}

/// Search seen addresses for `from:` / `to:` operator typeahead.
///
/// This queries `seen_addresses` (addresses observed in message headers),
/// not the synced contacts table.
pub fn search_seen_addresses_for_typeahead_sync(
    conn: &Connection,
    query: &str,
) -> Result<Vec<SeenAddressTypeaheadRow>, String> {
    let pattern = crate::db::make_like_pattern(query.trim());
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT display_name, email
             FROM seen_addresses
             WHERE (display_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
                    OR email LIKE ?1 ESCAPE '\\' COLLATE NOCASE)
             ORDER BY last_seen_at DESC
             LIMIT 10",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![pattern], |row| {
        Ok(SeenAddressTypeaheadRow {
            email: row.get("email")?,
            display_name: row.get("display_name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// An account row for search-operator typeahead.
#[derive(Debug, Clone)]
pub struct AccountTypeaheadRow {
    pub email: String,
    pub display_name: Option<String>,
    pub account_name: Option<String>,
}

/// Search accounts for `account:` operator typeahead.
pub fn search_accounts_for_typeahead_sync(
    conn: &Connection,
    query: &str,
) -> Result<Vec<AccountTypeaheadRow>, String> {
    let pattern = crate::db::make_like_pattern(query.trim());
    let mut stmt = conn
        .prepare(
            "SELECT email, display_name, account_name
             FROM accounts
             WHERE (email LIKE ?1 ESCAPE '\\' COLLATE NOCASE
                    OR display_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE
                    OR account_name LIKE ?1 ESCAPE '\\' COLLATE NOCASE)
             ORDER BY sort_order ASC
             LIMIT 10",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![pattern], |row| {
        Ok(AccountTypeaheadRow {
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            account_name: row.get("account_name")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

// ── Labels settings and explicit sidebar label groups ───────

/// A single raw per-account label that belongs to an explicit label group.
/// A single label entry in the settings Mail Rules > Labels list.
///
/// One row per `(account_id, label_id)` pair - settings shows raw provider
/// reality. Cross-account grouping is a sidebar concern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLabelRow {
    pub account_id: String,
    pub label_id: String,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
    pub has_color_override: bool,
    pub sort_order: i64,
    /// `labels.is_undeletable = 1`. Today covers Graph importance synth
    /// tags (`importance:high` / `importance:low`). Editable (name +
    /// colour) but the Delete action must be hidden.
    pub is_undeletable: bool,
}

/// Per-account section of the settings label list. Header text is the
/// account's display name; rows are that account's raw labels in sort_order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLabelsGroup {
    pub account_id: String,
    pub account_name: String,
    /// Optional account color hex (used for the section header chrome
    /// if the UI wants it - the settings list itself only uses the
    /// name today).
    pub account_color: Option<String>,
    pub labels: Vec<AccountLabelRow>,
}

/// Return all raw labels grouped by account, in account `sort_order`
/// then label `sort_order`. Drives the Mail Rules > Labels settings list.
pub fn query_labels_by_account(
    conn: &Connection,
) -> Result<Vec<AccountLabelsGroup>, String> {
    let mut acc_stmt = conn
        .prepare(
            "SELECT id, COALESCE(account_name, email) AS name, \
                    account_color \
             FROM accounts \
             WHERE is_active = 1 \
             ORDER BY sort_order, name",
        )
        .map_err(|e| e.to_string())?;
    let acc_rows = acc_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("name")?,
                row.get::<_, Option<String>>("account_color")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut groups: Vec<AccountLabelsGroup> = Vec::new();
    for r in acc_rows {
        let (account_id, name, color) = r.map_err(|e| e.to_string())?;
        groups.push(AccountLabelsGroup {
            account_id,
            account_name: name,
            account_color: color,
            labels: Vec::new(),
        });
    }

    let mut lbl_stmt = conn
        .prepare(
            "SELECT id, account_id, name, server_color_bg, server_color_fg, \
                    user_color_bg, user_color_fg, \
                    COALESCE(sort_order, 0) AS sort_order, \
                    COALESCE(is_undeletable, 0) AS is_undeletable \
             FROM labels \
             WHERE COALESCE(visible, 1) = 1 \
             ORDER BY account_id, sort_order, name",
        )
        .map_err(|e| e.to_string())?;
    let lbl_rows = lbl_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("account_id")?,
                row.get::<_, String>("name")?,
                row.get::<_, Option<String>>("server_color_bg")?,
                row.get::<_, Option<String>>("server_color_fg")?,
                row.get::<_, Option<String>>("user_color_bg")?,
                row.get::<_, Option<String>>("user_color_fg")?,
                row.get::<_, i64>("sort_order")?,
                row.get::<_, i64>("is_undeletable")? != 0,
            ))
        })
        .map_err(|e| e.to_string())?;

    for r in lbl_rows {
        let (label_id, account_id, name, server_color_bg, server_color_fg, user_color_bg, user_color_fg, sort_order, is_undeletable) =
            r.map_err(|e| e.to_string())?;

        let user_pair = label_colors::LabelStyleHex::from_optional_pair(
            user_color_bg.as_deref(),
            user_color_fg.as_deref(),
        )?;
        let has_color_override = user_pair.is_some();
        let server_pair = label_colors::LabelStyleHex::from_optional_pair(
            server_color_bg.as_deref(),
            server_color_fg.as_deref(),
        )?;

        let style = label_colors::resolve_label_color(
            &name,
            &account_id,
            user_pair,
            server_pair,
        );
        let bg = style.bg().to_owned();
        let fg = style.fg().to_owned();

        if let Some(group) = groups.iter_mut().find(|g| g.account_id == account_id) {
            group.labels.push(AccountLabelRow {
                account_id,
                label_id,
                name,
                color_bg: bg,
                color_fg: fg,
                has_color_override,
                sort_order,
                is_undeletable,
            });
        }
    }

    // Settings should not show empty per-account sections.
    groups.retain(|g| !g.labels.is_empty());

    Ok(groups)
}

/// One row in the Settings > Labels top section. Represents a user-created
/// `label_groups` row with its resolved colour and current member count.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsLabelGroupRow {
    pub id: i64,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
    pub member_count: i64,
}

/// Persist a new ordering for label groups. Each `(group_id, sort_order)`
/// pair is written in a single transaction. Drives drag-to-reorder in
/// Settings > Labels.
pub fn update_label_group_sort_order_sync(
    conn: &Connection,
    updates: &[(i64, i64)],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("label_group.reorder begin tx: {e}"))?;
    {
        let mut stmt = tx
            .prepare("UPDATE label_groups SET sort_order = ?1 WHERE id = ?2")
            .map_err(|e| e.to_string())?;
        for (id, order) in updates {
            stmt.execute(params![order, id])
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit()
        .map_err(|e| format!("label_group.reorder commit: {e}"))?;
    Ok(())
}

/// Members of one `label_groups` row as `(account_id, label_id)` pairs.
/// Used to populate the editor sheet on open.
pub fn query_label_group_members(
    conn: &Connection,
    group_id: i64,
) -> Result<Vec<(String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT account_id, label_id \
             FROM label_group_members \
             WHERE group_id = ?1 \
             ORDER BY account_id, label_id",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![group_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// All user-visible label groups, ordered by name, with their member counts.
/// Drives the top section of the Labels settings tab.
pub fn query_label_groups_for_settings(
    conn: &Connection,
) -> Result<Vec<SettingsLabelGroupRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT lg.id, lg.name, lg.color_bg, lg.color_fg, \
                    (SELECT COUNT(*) FROM label_group_members lgm \
                     WHERE lgm.group_id = lg.id) AS member_count \
             FROM label_groups lg \
             ORDER BY lg.sort_order ASC, lg.name COLLATE NOCASE ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        Ok(SettingsLabelGroupRow {
            id: row.get("id")?,
            name: row.get("name")?,
            color_bg: row.get("color_bg")?,
            color_fg: row.get("color_fg")?,
            member_count: row.get("member_count")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}
