//! IMAP NAMESPACE-based public folder discovery, permissions, and sync.
//!
//! Bridges IMAP shared namespaces (RFC 2342) to the provider-agnostic
//! `public_folders` table, enabling Dovecot/Cyrus shared folder access
//! alongside Exchange (EWS) public folders.

use serde::{Deserialize, Serialize};

use super::client::list_shared_folders;
use super::connection::{ImapSession, discover_myrights, discover_namespaces};
use super::types::{ImapFolder, NamespaceType};
use db::db::{ReadDbState, WriterPool};
use db::db::queries_extra::{
    PublicFolderItemRow, PublicFolderRow, get_public_folder_sync_depth,
    update_public_folder_rights, upsert_public_folder_items, upsert_public_folders,
};

// ── Types ────────────────────────────────────────────────────

/// A shared/public folder discovered via IMAP NAMESPACE.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapPublicFolder {
    pub path: String,
    pub display_name: String,
    pub namespace_type: NamespaceType,
    pub message_count: u32,
    pub unseen_count: u32,
}

/// Parsed IMAP ACL rights for a folder (RFC 4314).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImapFolderRights {
    /// `r` - read messages
    pub can_read: bool,
    /// `w` - write flags (other than \Seen and \Deleted)
    pub can_write: bool,
    /// `i` - insert/append messages
    pub can_insert: bool,
    /// `d` or `t` - delete messages
    pub can_delete: bool,
    /// `k` or `c` - create subfolders
    pub can_create: bool,
    /// `a` - administer (change ACLs)
    pub can_administer: bool,
}

/// Result of syncing items from an IMAP public folder.
#[derive(Debug, Default)]
pub struct PublicFolderSyncResult {
    pub folder_id: String,
    pub new_items: usize,
    pub updated_items: usize,
}

// ── Rights parsing ───────────────────────────────────────────

/// Parse an IMAP ACL rights string (e.g. `"lrswipcda"`) into structured flags.
pub fn parse_rights(rights: &str) -> ImapFolderRights {
    ImapFolderRights {
        can_read: rights.contains('r'),
        can_write: rights.contains('w'),
        can_insert: rights.contains('i'),
        can_delete: rights.contains('d') || rights.contains('t'),
        can_create: rights.contains('k') || rights.contains('c'),
        can_administer: rights.contains('a'),
    }
}

// ── Discovery ────────────────────────────────────────────────

/// Discover shared/public folders via IMAP NAMESPACE and persist them.
///
/// 1. Runs NAMESPACE to find shared prefixes
/// 2. Lists all folders under those prefixes (with STATUS counts)
/// 3. Upserts into the `public_folders` table
pub async fn discover_imap_public_folders(
    session: &mut ImapSession,
    db: &WriterPool,
    account_id: &str,
) -> Result<Vec<ImapPublicFolder>, String> {
    let namespace_info = discover_namespaces(session).await?;

    if namespace_info.shared.is_empty() && namespace_info.other_users.is_empty() {
        log::info!("IMAP NAMESPACE: no shared or other-users namespaces found");
        return Ok(Vec::new());
    }

    let shared_folders = list_shared_folders(session, &namespace_info).await?;

    if shared_folders.is_empty() {
        log::info!("IMAP: no selectable shared folders found under namespace prefixes");
        return Ok(Vec::new());
    }

    log::info!(
        "IMAP: discovered {} shared folder(s), persisting to public_folders",
        shared_folders.len()
    );

    // Build public folder representations
    let public_folders: Vec<ImapPublicFolder> = shared_folders
        .iter()
        .map(|f| ImapPublicFolder {
            path: f.path.clone(),
            display_name: f.name.clone(),
            namespace_type: f.namespace_type.clone().unwrap_or(NamespaceType::Shared),
            message_count: f.exists,
            unseen_count: f.unseen,
        })
        .collect();

    // Persist to DB
    persist_discovered_folders(db, account_id, &shared_folders).await?;

    Ok(public_folders)
}

// ── Permissions ──────────────────────────────────────────────

/// Check the current user's rights on a shared folder via MYRIGHTS (RFC 4314)
/// and update the `public_folders` row with the resolved permissions.
pub async fn check_folder_rights(
    session: &mut ImapSession,
    db: &WriterPool,
    account_id: &str,
    folder_path: &str,
) -> Result<ImapFolderRights, String> {
    let rights_str = discover_myrights(session, folder_path).await?;
    let rights = parse_rights(&rights_str);

    // Update public_folders row with resolved permissions
    let account_id = account_id.to_string();
    let folder_path = folder_path.to_string();
    let can_read = rights.can_read;
    let can_insert = rights.can_insert;
    let can_write = rights.can_write;
    let can_delete = rights.can_delete;

    db.with_write(move |conn| {
        update_public_folder_rights(conn, &account_id, &folder_path, can_read, can_insert, can_write, can_delete)
    })
    .await?;

    Ok(rights)
}

// ── Sync ─────────────────────────────────────────────────────

/// Sync a single IMAP public folder by fetching recent messages.
///
/// Since IMAP shared folders use standard SELECT/FETCH, this works like
/// regular IMAP sync but persists to `public_folder_items`.
pub async fn sync_imap_public_folder(
    session: &mut ImapSession,
    db: &ReadDbState,
    writer: &WriterPool,
    account_id: &str,
    folder_path: &str,
) -> Result<PublicFolderSyncResult, String> {
    use super::client::fetch_messages;

    let (last_sync_ts, _) = load_sync_state(db, account_id, folder_path).await?;
    let now = chrono::Utc::now().timestamp();

    // Both initial and incremental syncs use SEARCH SINCE with depth_days
    let depth_days = load_sync_depth_days(db, account_id, folder_path).await?;
    let since_date = chrono::Utc::now() - chrono::Duration::days(i64::from(depth_days));
    let since_str = since_date.format("%d-%b-%Y").to_string();

    if last_sync_ts.is_none() {
        log::info!(
            "IMAP public folder {folder_path}: initial sync, looking back {depth_days} days"
        );
    }

    // SEARCH SINCE to find relevant UIDs
    let search_result =
        super::client::search_folder(session, folder_path, Some(since_str.clone())).await?;

    if search_result.uids.is_empty() {
        log::info!("IMAP public folder {folder_path}: no messages matching SINCE {since_str}");
        save_sync_state(writer, account_id, folder_path, now).await?;
        return Ok(PublicFolderSyncResult {
            folder_id: folder_path.to_string(),
            ..Default::default()
        });
    }

    // Build UID set from search results and FETCH
    let uid_set = build_uid_set(&search_result.uids);
    let fetch_result = fetch_messages(session, folder_path, &uid_set).await?;

    log::info!(
        "IMAP public folder {folder_path}: fetched {} messages",
        fetch_result.messages.len()
    );

    // Persist messages to public_folder_items
    let new_items =
        upsert_public_folder_items_imap(writer, account_id, folder_path, &fetch_result.messages).await?;

    save_sync_state(writer, account_id, folder_path, now).await?;

    Ok(PublicFolderSyncResult {
        folder_id: folder_path.to_string(),
        new_items,
        updated_items: 0,
    })
}

// ── DB helpers ───────────────────────────────────────────────

/// Upsert discovered IMAP folders into the `public_folders` table.
///
/// Uses the decoded folder path as `folder_id` since IMAP folders don't have
/// opaque IDs like EWS - the path IS the identifier.
async fn persist_discovered_folders(
    db: &WriterPool,
    account_id: &str,
    folders: &[ImapFolder],
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let folders: Vec<ImapFolder> = folders.to_vec();

    db.with_write(move |conn| {
        let rows: Vec<PublicFolderRow> = folders
            .iter()
            .map(|f| {
                let parent_id = f
                    .path
                    .rsplit_once(&f.delimiter)
                    .map(|(parent, _)| parent.to_string());
                PublicFolderRow {
                    account_id: account_id.clone(),
                    folder_id: f.path.clone(),
                    parent_id,
                    display_name: f.name.clone(),
                    folder_class: "IPM.Note".to_string(),
                    unread_count: f.unseen,
                    total_count: f.exists,
                    // Permissions unknown until MYRIGHTS; assume readable
                    can_read: true,
                    can_create_items: false,
                    can_modify: false,
                    can_delete: false,
                }
            })
            .collect();
        upsert_public_folders(conn, &rows)
    })
    .await
}

/// Load sync state for a public folder.
async fn load_sync_state(
    db: &ReadDbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(Option<i64>, Option<i64>), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_read(move |conn| {
        let result = conn
            .query_row(
                "SELECT last_sync_timestamp, last_full_scan_at \
                 FROM public_folder_sync_state \
                 WHERE account_id = ?1 AND folder_id = ?2",
                rusqlite::params![account_id, folder_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>("last_sync_timestamp")?,
                        row.get::<_, Option<i64>>("last_full_scan_at")?,
                    ))
                },
            )
            .ok();

        Ok(result.unwrap_or((None, None)))
    })
    .await
}

/// Save sync state after a sync run.
async fn save_sync_state(
    db: &WriterPool,
    account_id: &str,
    folder_id: &str,
    last_sync_timestamp: i64,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_write(move |conn| {
        conn.execute(
            "INSERT INTO public_folder_sync_state (account_id, folder_id, last_sync_timestamp) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               last_sync_timestamp = excluded.last_sync_timestamp",
            rusqlite::params![account_id, folder_id, last_sync_timestamp],
        )
        .map_err(|e| format!("save_sync_state: {e}"))?;
        Ok(())
    })
    .await
}

/// Load sync_depth_days from `public_folder_pins` for a folder. Defaults to 30.
async fn load_sync_depth_days(
    db: &ReadDbState,
    account_id: &str,
    folder_id: &str,
) -> Result<i32, String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_read(move |conn| get_public_folder_sync_depth(conn, &account_id, &folder_id))
        .await
}

/// Upsert fetched messages into `public_folder_items`. Returns count of new items.
async fn upsert_public_folder_items_imap(
    db: &WriterPool,
    account_id: &str,
    folder_id: &str,
    messages: &[super::types::ImapMessage],
) -> Result<usize, String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    let messages: Vec<super::types::ImapMessage> = messages.to_vec();

    db.with_write(move |conn| {
        // Build rows, skipping messages without a stable Message-ID
        let rows: Vec<PublicFolderItemRow> = messages
            .iter()
            .filter_map(|msg| {
                let item_id = msg.message_id.as_deref().unwrap_or("").to_string();
                if item_id.is_empty() {
                    return None;
                }
                Some(PublicFolderItemRow {
                    account_id: account_id.clone(),
                    folder_id: folder_id.clone(),
                    item_id,
                    change_key: None,
                    subject: msg.subject.clone(),
                    sender_email: msg.from_address.clone(),
                    sender_name: msg.from_name.clone(),
                    received_at: Some(msg.date),
                    body_preview: msg.snippet.clone(),
                    is_read: msg.is_read,
                    item_class: "IPM.Note".to_string(),
                })
            })
            .collect();

        let (new_count, _updated_count) = upsert_public_folder_items(conn, &rows)?;
        Ok(new_count)
    })
    .await
}

/// Build a compact UID set string from a sorted list of UIDs (e.g. "1,3,5:10,15").
fn build_uid_set(uids: &[u32]) -> String {
    if uids.is_empty() {
        return String::new();
    }

    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut parts: Vec<String> = Vec::new();
    let mut range_start = sorted[0];
    let mut range_end = sorted[0];

    for &uid in &sorted[1..] {
        if uid == range_end + 1 {
            range_end = uid;
        } else {
            if range_start == range_end {
                parts.push(range_start.to_string());
            } else {
                parts.push(format!("{range_start}:{range_end}"));
            }
            range_start = uid;
            range_end = uid;
        }
    }

    // Push the last range
    if range_start == range_end {
        parts.push(range_start.to_string());
    } else {
        parts.push(format!("{range_start}:{range_end}"));
    }

    parts.join(",")
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- Rights parsing ----------

    #[test]
    fn parse_full_rights() {
        let rights = parse_rights("lrswipcda");
        assert!(rights.can_read);
        assert!(rights.can_write);
        assert!(rights.can_insert);
        assert!(rights.can_delete);
        assert!(rights.can_create);
        assert!(rights.can_administer);
    }

    #[test]
    fn parse_read_only_rights() {
        let rights = parse_rights("lr");
        assert!(rights.can_read);
        assert!(!rights.can_write);
        assert!(!rights.can_insert);
        assert!(!rights.can_delete);
        assert!(!rights.can_create);
        assert!(!rights.can_administer);
    }

    #[test]
    fn parse_empty_rights() {
        let rights = parse_rights("");
        assert!(!rights.can_read);
        assert!(!rights.can_write);
        assert!(!rights.can_insert);
        assert!(!rights.can_delete);
        assert!(!rights.can_create);
        assert!(!rights.can_administer);
    }

    #[test]
    fn parse_delete_via_t_flag() {
        // RFC 4314 't' is the modern replacement for 'd'
        let rights = parse_rights("lrst");
        assert!(rights.can_read);
        assert!(rights.can_delete);
        assert!(!rights.can_write);
    }

    #[test]
    fn parse_create_via_k_flag() {
        // RFC 4314 'k' is the modern replacement for 'c'
        let rights = parse_rights("lrsk");
        assert!(rights.can_read);
        assert!(rights.can_create);
        assert!(!rights.can_administer);
    }

    #[test]
    fn parse_rights_with_both_legacy_and_modern() {
        // Some servers send both old and new flags
        let rights = parse_rights("lrswipcdteka");
        assert!(rights.can_read);
        assert!(rights.can_write);
        assert!(rights.can_insert);
        assert!(rights.can_delete);
        assert!(rights.can_create);
        assert!(rights.can_administer);
    }

    // ---------- UID set building ----------

    #[test]
    fn build_uid_set_single() {
        assert_eq!(build_uid_set(&[42]), "42");
    }

    #[test]
    fn build_uid_set_consecutive() {
        assert_eq!(build_uid_set(&[1, 2, 3, 4, 5]), "1:5");
    }

    #[test]
    fn build_uid_set_gaps() {
        assert_eq!(build_uid_set(&[1, 3, 5, 6, 7, 10]), "1,3,5:7,10");
    }

    #[test]
    fn build_uid_set_unsorted() {
        assert_eq!(build_uid_set(&[5, 3, 1, 2, 4]), "1:5");
    }

    #[test]
    fn build_uid_set_duplicates() {
        assert_eq!(build_uid_set(&[1, 1, 2, 2, 3]), "1:3");
    }

    #[test]
    fn build_uid_set_empty() {
        assert_eq!(build_uid_set(&[]), "");
    }

    // ---------- Folder persistence mapping ----------

    #[test]
    fn parent_id_from_path() {
        // Simulate the parent_id derivation logic used in persist_discovered_folders
        let path = "Shared/Team/Inbox";
        let delimiter = "/";
        let parent_id = path
            .rsplit_once(delimiter)
            .map(|(parent, _)| parent.to_string());
        assert_eq!(parent_id.as_deref(), Some("Shared/Team"));
    }

    #[test]
    fn parent_id_top_level() {
        let path = "SharedFolder";
        let delimiter = "/";
        let parent_id = path
            .rsplit_once(delimiter)
            .map(|(parent, _)| parent.to_string());
        assert_eq!(parent_id, None);
    }

    #[test]
    fn parent_id_dot_delimiter() {
        let path = "Shared.Team.Inbox";
        let delimiter = ".";
        let parent_id = path
            .rsplit_once(delimiter)
            .map(|(parent, _)| parent.to_string());
        assert_eq!(parent_id.as_deref(), Some("Shared.Team"));
    }

    // ---------- Namespace type classification ----------

    #[test]
    fn imap_public_folder_shared_type() {
        let f = ImapPublicFolder {
            path: "Shared/team-inbox".to_string(),
            display_name: "team-inbox".to_string(),
            namespace_type: NamespaceType::Shared,
            message_count: 42,
            unseen_count: 5,
        };
        assert_eq!(f.namespace_type, NamespaceType::Shared);
        assert_eq!(f.message_count, 42);
    }

    #[test]
    fn imap_public_folder_other_users_type() {
        let f = ImapPublicFolder {
            path: "Other Users/bob/INBOX".to_string(),
            display_name: "INBOX".to_string(),
            namespace_type: NamespaceType::OtherUsers,
            message_count: 100,
            unseen_count: 10,
        };
        assert_eq!(f.namespace_type, NamespaceType::OtherUsers);
    }

    // ---------- DB integration tests ----------

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE public_folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                parent_id TEXT,
                display_name TEXT NOT NULL,
                folder_class TEXT,
                unread_count INTEGER NOT NULL DEFAULT 0,
                total_count INTEGER NOT NULL DEFAULT 0,
                can_create_items INTEGER NOT NULL DEFAULT 0,
                can_modify INTEGER NOT NULL DEFAULT 0,
                can_delete INTEGER NOT NULL DEFAULT 0,
                can_read INTEGER NOT NULL DEFAULT 1,
                UNIQUE(account_id, folder_id)
            );
            CREATE TABLE public_folder_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                change_key TEXT,
                subject TEXT,
                sender_email TEXT,
                sender_name TEXT,
                received_at INTEGER,
                body_preview TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                item_class TEXT NOT NULL DEFAULT 'IPM.Note',
                UNIQUE(account_id, item_id)
            );
            CREATE TABLE public_folder_pins (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                sync_enabled INTEGER NOT NULL DEFAULT 1,
                sync_depth_days INTEGER NOT NULL DEFAULT 30,
                last_sync_at INTEGER,
                UNIQUE(account_id, folder_id)
            );
            CREATE TABLE public_folder_sync_state (
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                last_sync_timestamp INTEGER,
                last_full_scan_at INTEGER,
                PRIMARY KEY(account_id, folder_id)
            );
            "#,
        )
        .expect("create tables");
        conn
    }

    #[test]
    fn persist_folder_upsert() {
        let conn = setup_test_db();

        // Insert a shared folder
        conn.execute(
            "INSERT INTO public_folders \
             (account_id, folder_id, parent_id, display_name, folder_class, \
              unread_count, total_count, can_read) \
             VALUES ('acc1', 'Shared/team', 'Shared', 'team', 'IPM.Note', 5, 42, 1) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               display_name = excluded.display_name, \
               unread_count = excluded.unread_count, \
               total_count = excluded.total_count",
            [],
        )
        .expect("insert");

        let (name, total): (String, i32) = conn
            .query_row(
                "SELECT display_name, total_count FROM public_folders \
                 WHERE account_id = 'acc1' AND folder_id = 'Shared/team'",
                [],
                |row| Ok((row.get("display_name")?, row.get("total_count")?)),
            )
            .expect("query");
        assert_eq!(name, "team");
        assert_eq!(total, 42);

        // Upsert with updated counts
        conn.execute(
            "INSERT INTO public_folders \
             (account_id, folder_id, parent_id, display_name, folder_class, \
              unread_count, total_count, can_read) \
             VALUES ('acc1', 'Shared/team', 'Shared', 'team', 'IPM.Note', 10, 99, 1) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               display_name = excluded.display_name, \
               unread_count = excluded.unread_count, \
               total_count = excluded.total_count",
            [],
        )
        .expect("upsert");

        let (name2, total2): (String, i32) = conn
            .query_row(
                "SELECT display_name, total_count FROM public_folders \
                 WHERE account_id = 'acc1' AND folder_id = 'Shared/team'",
                [],
                |row| Ok((row.get("display_name")?, row.get("total_count")?)),
            )
            .expect("query2");
        assert_eq!(name2, "team");
        assert_eq!(total2, 99);

        // Should still be 1 row
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folders WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn rights_update_in_db() {
        let conn = setup_test_db();

        // Insert folder with default permissions
        conn.execute(
            "INSERT INTO public_folders \
             (account_id, folder_id, display_name, can_read, can_create_items, can_modify, can_delete) \
             VALUES ('acc1', 'Shared/team', 'team', 1, 0, 0, 0)",
            [],
        )
        .expect("insert");

        // Simulate MYRIGHTS update
        let rights = parse_rights("lrswipcd");
        conn.execute(
            "UPDATE public_folders \
             SET can_read = ?3, can_create_items = ?4, can_modify = ?5, can_delete = ?6 \
             WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![
                "acc1",
                "Shared/team",
                rights.can_read as i32,
                rights.can_insert as i32,
                rights.can_write as i32,
                rights.can_delete as i32,
            ],
        )
        .expect("update rights");

        let (can_read, can_create, can_modify, can_delete): (i32, i32, i32, i32) = conn
            .query_row(
                "SELECT can_read, can_create_items, can_modify, can_delete \
                 FROM public_folders WHERE account_id = 'acc1' AND folder_id = 'Shared/team'",
                [],
                |row| {
                    Ok((
                        row.get("can_read")?,
                        row.get("can_create_items")?,
                        row.get("can_modify")?,
                        row.get("can_delete")?,
                    ))
                },
            )
            .expect("query rights");

        assert_eq!(can_read, 1);
        assert_eq!(can_create, 1); // 'i' right
        assert_eq!(can_modify, 1); // 'w' right
        assert_eq!(can_delete, 1); // 'd' right
    }
}
