use std::collections::HashMap;

use crate::db::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::queries::get_labels;
use crate::db::types::{AccountScope, DbSmartFolder};
use crate::provider::folder_roles::SYSTEM_FOLDER_ROLES;

use crate::db::from_row::FromRow;

use crate::db::queries_extra::scoped_queries::{get_draft_count_with_local, get_unread_counts_by_folder};

// ── Types ───────────────────────────────────────────────────

/// The kind of folder shown in the sidebar navigation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FolderKind {
    /// A well-known system folder (Inbox, Sent, Drafts, etc.).
    Universal,
    /// A user-defined smart folder backed by a saved query.
    SmartFolder,
    /// A user-created provider folder specific to one account
    /// (container semantics - `label_kind = 'container'` in the DB).
    AccountFolder,
    /// A user-visible label - Exchange category, IMAP keyword, JMAP keyword,
    /// or Gmail user label (`label_kind = 'tag'` in the DB).
    AccountLabel,
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
    pub unread_count: i64,
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
    /// Resolved background color for the label dot/chip. `None` for non-label rows.
    /// Always set for `AccountLabel`: either the synced `color_bg` or a deterministic
    /// fallback from the preset palette via `label_colors::resolve_label_color`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_bg: Option<String>,
    /// Resolved foreground color, paired with `color_bg`. `None` for non-label rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_fg: Option<String>,
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

    if let AccountScope::Single(account_id) = scope {
        folders.extend(build_account_folders(conn, account_id).map_err(|e| {
            log::error!("Failed to build account folders for {account_id}: {e}");
            e
        })?);
        folders.extend(build_account_labels(conn, account_id).map_err(|e| {
            log::error!("Failed to build account labels for {account_id}: {e}");
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
                unread_count,
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
    let all_labels = get_labels(conn, account_id)?;
    let system_ids = system_label_ids();
    let unread_by_label = get_label_unread_counts(conn, account_id)?;

    Ok(all_labels
        .into_iter()
        .filter(|label| !system_ids.contains(&label.id.as_str()))
        .filter(|label| label.visible)
        // Only container-type rows here. Labels come from
        // build_account_labels() to avoid duplication.
        .filter(|label| label.label_kind != "tag")
        .map(|label| {
            let unread_count = unread_by_label.get(&label.id).copied().unwrap_or(0);

            let rights = rights_from_label(&label);

            // If parent is a system folder (INBOX, SENT, etc.), treat as
            // root - system folders are rendered in the universal section,
            // not in the label tree. Without this, children of system
            // folders become orphans in the tree and get promoted to
            // depth-0 by the orphan recovery path.
            let parent_id = label
                .parent_label_id
                .filter(|pid| !system_ids.contains(&pid.as_str()));

            NavigationFolder {
                is_subscribed: label.is_subscribed,
                id: label.id,
                name: label.name,
                folder_kind: FolderKind::AccountFolder,
                unread_count,
                account_id: Some(label.account_id),
                parent_id,
                query: None,
                rights,
                color_bg: None,
                color_fg: None,
            }
        })
        .collect())
}

/// Account-specific labels.
fn build_account_labels(conn: &Connection, account_id: &str) -> Result<Vec<NavigationFolder>, String> {
    let all_labels = get_labels(conn, account_id)?;
    let unread_by_label = get_label_unread_counts(conn, account_id)?;

    Ok(all_labels
        .into_iter()
        .filter(|label| label.visible)
        .filter(|label| label.label_kind == "tag")
        .map(|label| {
            let unread_count = unread_by_label.get(&label.id).copied().unwrap_or(0);
            let rights = rights_from_label(&label);
            let (bg, fg) = label_colors::resolve_label_color(
                &label.name,
                &label.account_id,
                None,
                label.color_bg.as_deref(),
                label.color_fg.as_deref(),
            );
            let bg = bg.to_owned();
            let fg = fg.to_owned();

            NavigationFolder {
                is_subscribed: label.is_subscribed,
                id: label.id,
                name: label.name,
                folder_kind: FolderKind::AccountLabel,
                unread_count,
                account_id: Some(label.account_id),
                parent_id: None,
                query: None,
                rights,
                color_bg: Some(bg),
                color_fg: Some(fg),
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
               AND t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0
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
    let all_labels = get_labels(conn, account_id)?;
    let system_ids = system_label_ids();

    // Unread counts for labels, scoped to this shared mailbox
    let mut unread_stmt = conn
        .prepare(
            "SELECT tl.label_id, COUNT(*) AS unread_count
             FROM threads t
             INNER JOIN thread_labels tl
               ON tl.account_id = t.account_id AND tl.thread_id = t.id
             WHERE t.account_id = ?1 AND t.shared_mailbox_id = ?2
               AND t.is_read = 0
             GROUP BY tl.label_id",
        )
        .map_err(|e| e.to_string())?;
    let unread_by_label: HashMap<String, i64> = unread_stmt
        .query_map(params![account_id, mailbox_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .collect();

    // Universal folders with shared-mailbox-scoped unread counts
    let mut folders: Vec<NavigationFolder> = SIDEBAR_UNIVERSAL_FOLDERS
        .iter()
        .map(|(id, name)| {
            let unread = unread_by_label.get(*id).copied().unwrap_or(0);
            NavigationFolder {
                id: (*id).to_owned(),
                name: (*name).to_owned(),
                folder_kind: FolderKind::Universal,
                unread_count: unread,
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

    // Account labels (non-system, visible) with shared-mailbox-scoped counts
    let label_folders: Vec<NavigationFolder> = all_labels
        .into_iter()
        .filter(|l| !system_ids.contains(&l.id.as_str()) && l.visible)
        .map(|label| {
            let unread = unread_by_label.get(&label.id).copied().unwrap_or(0);
            let rights = rights_from_label(&label);
            let parent_id = label
                .parent_label_id
                .filter(|pid| !system_ids.contains(&pid.as_str()));
            let kind = if label.label_kind == "tag" {
                FolderKind::AccountLabel
            } else {
                FolderKind::AccountFolder
            };
            let (color_bg, color_fg) = if matches!(kind, FolderKind::AccountLabel) {
                let (bg, fg) = label_colors::resolve_label_color(
                    &label.name,
                    &label.account_id,
                    None,
                    label.color_bg.as_deref(),
                    label.color_fg.as_deref(),
                );
                (Some(bg.to_owned()), Some(fg.to_owned()))
            } else {
                (None, None)
            };
            NavigationFolder {
                is_subscribed: label.is_subscribed,
                id: label.id,
                name: label.name,
                folder_kind: kind,
                unread_count: unread,
                account_id: Some(label.account_id),
                parent_id,
                query: None,
                rights,
                color_bg,
                color_fg,
            }
        })
        .collect();
    folders.extend(label_folders);

    Ok(NavigationState {
        scope: AccountScope::Single(account_id.to_string()),
        folders,
    })
}

/// Extract mailbox rights from a `DbLabel` into a `MailboxRightsInfo`.
///
/// Returns `None` if no rights are set (all fields are `None`), meaning
/// the provider doesn't supply rights data for this label.
fn rights_from_label(label: &crate::db::types::DbLabel) -> Option<MailboxRightsInfo> {
    label.right_read?;
    Some(MailboxRightsInfo {
        may_read_items: label.right_read,
        may_add_items: label.right_add,
        may_remove_items: label.right_remove,
        may_set_seen: label.right_set_seen,
        may_set_keywords: label.right_set_keywords,
        may_create_child: label.right_create_child,
        may_rename: label.right_rename,
        may_delete: label.right_delete,
        may_submit: label.right_submit,
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

// ── Cross-account labels (settings + sidebar section 4) ─────

/// A single row in the per-account backing list of a cross-account label.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelBacking {
    pub account_id: String,
    pub label_id: String,
    pub display_name: String,
}

/// A label as the user sees it: one normalized name, optionally backed by
/// rows on multiple accounts. The canonical entry point for sidebar
/// section 4 and the Mail Rules > Labels settings list.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossAccountLabel {
    /// `LOWER(TRIM(name))`. The grouping key and the override-table key.
    pub normalized_name: String,
    /// Whatever display form the user typed. If backings disagree on
    /// casing or whitespace, we pick the first row's display name.
    pub display_name: String,
    pub color_bg: String,
    pub color_fg: String,
    /// True if this label's color came from `label_color_overrides`.
    pub has_color_override: bool,
    /// Per-account rows that contribute to this label.
    pub backing: Vec<LabelBacking>,
    /// Sum of unread thread counts across all backings.
    pub unread_count: i64,
}

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
}

/// Per-account section of the settings label list. Header text is the
/// account's display name; rows are that account's tag-type labels in
/// sort_order.
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

/// Return all tag-type labels grouped by account, in account `sort_order`
/// then label `sort_order`. Drives the Mail Rules > Labels settings list.
pub fn query_labels_by_account(
    conn: &Connection,
) -> Result<Vec<AccountLabelsGroup>, String> {
    let overrides = load_label_color_overrides(conn)?;

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
            "SELECT id, account_id, name, color_bg, color_fg, \
                    COALESCE(sort_order, 0) AS sort_order \
             FROM labels \
             WHERE label_kind = 'tag' AND COALESCE(visible, 1) = 1 \
             ORDER BY account_id, sort_order, name",
        )
        .map_err(|e| e.to_string())?;
    let lbl_rows = lbl_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("account_id")?,
                row.get::<_, String>("name")?,
                row.get::<_, Option<String>>("color_bg")?,
                row.get::<_, Option<String>>("color_fg")?,
                row.get::<_, i64>("sort_order")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    for r in lbl_rows {
        let (label_id, account_id, name, color_bg, color_fg, sort_order) =
            r.map_err(|e| e.to_string())?;

        let normalized = name.trim().to_lowercase();
        let override_pair = overrides
            .get(&normalized)
            .map(|(b, f)| (b.as_str(), f.as_str()));
        let has_color_override = override_pair.is_some();

        let (bg, fg) = label_colors::resolve_label_color(
            &name,
            &account_id,
            override_pair,
            color_bg.as_deref(),
            color_fg.as_deref(),
        );
        let bg = bg.to_owned();
        let fg = fg.to_owned();

        if let Some(group) = groups.iter_mut().find(|g| g.account_id == account_id) {
            group.labels.push(AccountLabelRow {
                account_id,
                label_id,
                name,
                color_bg: bg,
                color_fg: fg,
                has_color_override,
                sort_order,
            });
        }
    }

    // Settings should not show empty per-account sections.
    groups.retain(|g| !g.labels.is_empty());

    Ok(groups)
}

/// Return all tag-type labels grouped by normalized name across all accounts,
/// sorted alphabetically. Used by sidebar section 4 and the settings tab.
///
/// Resolution rules per `docs/labels-unification/problem-statement.md`:
/// - Grouping key is `LOWER(TRIM(name))`.
/// - Color: override > synced consensus (exactly one non-null pair across
///   backings) > hash fallback.
/// - Unread count: SUM across backings of threads with `is_read = 0`.
pub fn query_visible_labels(conn: &Connection) -> Result<Vec<CrossAccountLabel>, String> {
    let overrides = load_label_color_overrides(conn)?;
    let unread = load_cross_account_unread_by_normalized_name(conn)?;

    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, name, color_bg, color_fg, \
             LOWER(TRIM(name)) AS normalized \
             FROM labels \
             WHERE label_kind = 'tag' AND COALESCE(visible, 1) = 1 \
             ORDER BY normalized, account_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("account_id")?,
                row.get::<_, String>("name")?,
                row.get::<_, Option<String>>("color_bg")?,
                row.get::<_, Option<String>>("color_fg")?,
                row.get::<_, String>("normalized")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut groups: HashMap<String, CrossAccountLabel> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut synced_pairs: HashMap<String, Vec<(String, String)>> = HashMap::new();

    for r in rows {
        let (label_id, account_id, name, color_bg, color_fg, normalized) =
            r.map_err(|e| e.to_string())?;

        if !groups.contains_key(&normalized) {
            order.push(normalized.clone());
            groups.insert(
                normalized.clone(),
                CrossAccountLabel {
                    normalized_name: normalized.clone(),
                    display_name: name.clone(),
                    color_bg: String::new(),
                    color_fg: String::new(),
                    has_color_override: false,
                    backing: Vec::new(),
                    unread_count: *unread.get(&normalized).unwrap_or(&0),
                },
            );
        }

        let group = groups.get_mut(&normalized).expect("just inserted");
        group.backing.push(LabelBacking {
            account_id: account_id.clone(),
            label_id,
            display_name: name.clone(),
        });

        if let (Some(bg), Some(fg)) = (color_bg, color_fg) {
            synced_pairs
                .entry(normalized.clone())
                .or_default()
                .push((bg, fg));
        }
    }

    for normalized in &order {
        let group = groups.get_mut(normalized).expect("in order");
        let override_pair = overrides
            .get(normalized)
            .map(|(b, f)| (b.as_str(), f.as_str()));

        let synced = synced_pairs.get(normalized).and_then(|pairs| {
            let mut distinct: Vec<&(String, String)> = Vec::new();
            for p in pairs {
                if !distinct.contains(&p) {
                    distinct.push(p);
                }
            }
            (distinct.len() == 1).then(|| distinct[0].clone())
        });

        let synced_ref = synced.as_ref().map(|(b, f)| (b.as_str(), f.as_str()));

        let (bg, fg) = label_colors::resolve_label_color(
            &group.display_name,
            "", // namespace empty -> grouping is by name, not per-account
            override_pair,
            synced_ref.map(|(b, _)| b),
            synced_ref.map(|(_, f)| f),
        );
        group.color_bg = bg.to_owned();
        group.color_fg = fg.to_owned();
        group.has_color_override = override_pair.is_some();
    }

    Ok(order
        .into_iter()
        .map(|n| groups.remove(&n).expect("present"))
        .collect())
}

fn load_label_color_overrides(
    conn: &Connection,
) -> Result<HashMap<String, (String, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT LOWER(TRIM(label_name)) AS normalized, color_bg, color_fg \
             FROM label_color_overrides",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("normalized")?,
                row.get::<_, String>("color_bg")?,
                row.get::<_, String>("color_fg")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for r in rows {
        let (norm, bg, fg) = r.map_err(|e| e.to_string())?;
        map.insert(norm, (bg, fg));
    }
    Ok(map)
}

fn load_cross_account_unread_by_normalized_name(
    conn: &Connection,
) -> Result<HashMap<String, i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT LOWER(TRIM(l.name)) AS normalized, \
                    COUNT(DISTINCT t.account_id || ':' || t.id) AS unread \
             FROM threads t \
             INNER JOIN thread_labels tl \
               ON tl.account_id = t.account_id AND tl.thread_id = t.id \
             INNER JOIN labels l \
               ON l.account_id = tl.account_id AND l.id = tl.label_id \
             WHERE l.label_kind = 'tag' \
               AND COALESCE(l.visible, 1) = 1 \
               AND t.is_read = 0 \
             GROUP BY normalized",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>("normalized")?,
                row.get::<_, i64>("unread")?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut map = HashMap::new();
    for r in rows {
        let (norm, count) = r.map_err(|e| e.to_string())?;
        map.insert(norm, count);
    }
    Ok(map)
}
