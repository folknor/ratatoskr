//! B6a verification: the role-mapping unit test and the
//! `containers_persist_equals_legacy` golden.
//!
//! The golden is a CHECKED-IN snapshot (not a live legacy call - the legacy
//! `sync_*_folder_map` functions are deleted in the same landing). It pins
//! the `folders` / `labels` rows (ids, names, parents, `is_undeletable`,
//! `server_color_*`) `sync_containers` produces from a per-provider
//! `containers_list` fixture against what the legacy passes produced from
//! the equivalent payload, including the Gmail `CATEGORY_*` / `IMPORTANT` /
//! `CHAT` system-label-as-folder rows and the `user_color_*` / `importance:*`
//! preservation semantics.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use bifrost_types::{
    Container, ContainerId, ContainerKind, ContainerStyle, FolderRole, ProtocolKind, Provenance,
};
use common::types::{FolderKind, SystemFolderId};
use rusqlite::Connection;

use super::{build_container_rows, folder_kind_for, role_to_system_folder_id};

const ACCOUNT: &str = "acct-1";

fn provenance(provider: ProtocolKind, kind: ContainerKind, native: &str) -> Provenance {
    Provenance {
        provider,
        kind,
        native: native.to_string(),
    }
}

/// A Gmail label/system container (Gmail is label-shaped on the wire).
fn gmail(
    native: &str,
    name: &str,
    role: Option<FolderRole>,
    system: bool,
    style: Option<ContainerStyle>,
) -> Container {
    Container::new(
        ContainerId(native.to_string()),
        ContainerKind::Label,
        role,
        provenance(ProtocolKind::Gmail, ContainerKind::Label, native),
        name.to_string(),
        None,
    )
    .with_system(system)
    .with_style(style)
}

/// A folder-shaped container (Graph / JMAP / IMAP).
fn folder(
    provider: ProtocolKind,
    native: &str,
    name: &str,
    role: Option<FolderRole>,
    parent: Option<&str>,
) -> Container {
    Container::new(
        ContainerId(native.to_string()),
        ContainerKind::Folder,
        role,
        provenance(provider, ContainerKind::Folder, native),
        name.to_string(),
        parent.map(|p| ContainerId(p.to_string())),
    )
    // JMAP marks role-bearing mailboxes system; the others leave it false.
    .with_system(matches!(provider, ProtocolKind::Jmap) && role.is_some())
}

fn find_folder<'a>(
    rows: &'a [db::db::queries_extra::FolderWriteRow],
    id: &str,
) -> &'a db::db::queries_extra::FolderWriteRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("expected folder row {id}; have {:?}", ids(rows)))
}

fn ids(rows: &[db::db::queries_extra::FolderWriteRow]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

// ── Role-mapping unit test ──────────────────────────────

#[test]
fn container_role_maps_to_canonical_id() {
    // Every known bifrost FolderRole maps to its glossary canonical id.
    // `FolderRole` is `#[non_exhaustive]`, so a wildcard arm is forced; the
    // pairs below pin every variant the migration knows.
    let pairs = [
        (FolderRole::Inbox, SystemFolderId::Inbox, "INBOX"),
        (FolderRole::Sent, SystemFolderId::Sent, "SENT"),
        (FolderRole::Drafts, SystemFolderId::Draft, "DRAFT"),
        (FolderRole::Archive, SystemFolderId::Archive, "archive"),
        (FolderRole::Trash, SystemFolderId::Trash, "TRASH"),
        (FolderRole::Spam, SystemFolderId::Spam, "SPAM"),
    ];
    for (role, system, canonical) in pairs {
        assert_eq!(
            role_to_system_folder_id(role),
            Some(system),
            "role {role:?} must map to {system:?}"
        );
        assert_eq!(
            FolderKind::System(system).storage_id(),
            canonical,
            "system {system:?} must carry storage id {canonical}"
        );
    }

    // Prefixed-native fallback per provider for a non-system (role == None)
    // container, pinned against the glossary Identity table.
    let graph = folder(ProtocolKind::Graph, "GUID9", "Work", None, None);
    assert_eq!(folder_kind_for(&graph).unwrap().storage_id(), "graph-GUID9");

    let jmap = folder(ProtocolKind::Jmap, "mb9", "Projects", None, None);
    assert_eq!(folder_kind_for(&jmap).unwrap().storage_id(), "jmap-mb9");

    let imap = folder(ProtocolKind::Imap, "INBOX/Work", "Work", None, None);
    assert_eq!(
        folder_kind_for(&imap).unwrap().storage_id(),
        "folder-INBOX/Work"
    );

    // Gmail user label keeps its native id (no prefix); Gmail non-role
    // system labels resolve to their canonical / GmailSystem storage id.
    let category = gmail("CATEGORY_PROMOTIONS", "Promotions", None, true, None);
    assert_eq!(
        folder_kind_for(&category).unwrap().storage_id(),
        "CATEGORY_PROMOTIONS"
    );
    let important = gmail("IMPORTANT", "Important", None, true, None);
    assert_eq!(
        folder_kind_for(&important).unwrap().storage_id(),
        "IMPORTANT"
    );
}

// ── The equality golden ─────────────────────────────────

fn gmail_fixture() -> Vec<Container> {
    vec![
        // Synthetic bifrost archive (Gmail models archive as the absence of
        // INBOX). Role-bearing, so it lands as a folder.
        gmail("archive", "Archive", Some(FolderRole::Archive), false, None),
        gmail("INBOX", "Inbox", Some(FolderRole::Inbox), true, None),
        gmail("SENT", "Sent", Some(FolderRole::Sent), true, None),
        // Non-role system labels: role alone cannot route these, `system`
        // does (the § 2.5 blocker the golden exists to catch).
        gmail("CATEGORY_PROMOTIONS", "Promotions", None, true, None),
        gmail("IMPORTANT", "Important", None, true, None),
        gmail("CHAT", "Chat", None, true, None),
        // Message-state ids are filtered before the folder/label split, so a
        // Gmail STARRED system label never lands as a container row.
        gmail("STARRED", "Starred", None, true, None),
        gmail("UNREAD", "Unread", None, true, None),
        // User label with a server color.
        gmail(
            "Label_42",
            "Work",
            None,
            false,
            Some(ContainerStyle::new("#16a766", "#ffffff")),
        ),
    ]
}

#[test]
fn containers_persist_equals_legacy() {
    // Gmail: the system-label-as-folder split + colors + message-state filter.
    let (folders, labels, folder_map) = build_container_rows(ACCOUNT, &gmail_fixture()).unwrap();

    // Folder rows: every system label (role-bearing OR `system`) lands here.
    let mut folder_ids = ids(&folders);
    folder_ids.sort_unstable();
    assert_eq!(
        folder_ids,
        [
            "CATEGORY_PROMOTIONS",
            "CHAT",
            "IMPORTANT",
            "INBOX",
            "SENT",
            "archive"
        ],
        "Gmail CATEGORY_*/IMPORTANT/CHAT must land as folders, STARRED/UNREAD filtered"
    );
    for row in &folders {
        assert!(
            row.is_undeletable,
            "Gmail system folder {} must be undeletable",
            row.id
        );
        assert_eq!(row.account_id, ACCOUNT);
    }
    assert_eq!(
        find_folder(&folders, "CATEGORY_PROMOTIONS").name,
        "Promotions"
    );

    // Label rows: only the user label, carrying its server color, deletable.
    assert_eq!(labels.len(), 1, "only the Gmail user label is a label row");
    let work = &labels[0];
    assert_eq!(work.id, "Label_42");
    assert_eq!(work.name, "Work");
    assert_eq!(work.server_color_bg.as_deref(), Some("#16a766"));
    assert_eq!(work.server_color_fg.as_deref(), Some("#ffffff"));
    assert_eq!(work.user_color_bg, None);
    assert!(!work.is_undeletable);

    // Folder map: native-id keyed, folders only.
    assert_eq!(folder_map.len(), 6);
    assert_eq!(
        folder_map.get("INBOX"),
        Some(&FolderKind::System(SystemFolderId::Inbox))
    );
    assert!(!folder_map.contains_key("Label_42"));
    assert!(!folder_map.contains_key("STARRED"));

    // Graph: hierarchy + two-pass parent resolution.
    let graph = vec![
        folder(
            ProtocolKind::Graph,
            "g-inbox",
            "Inbox",
            Some(FolderRole::Inbox),
            None,
        ),
        folder(ProtocolKind::Graph, "g-work", "Work", None, Some("g-inbox")),
        folder(ProtocolKind::Graph, "g-sub", "Sub", None, Some("g-work")),
    ];
    let (gfolders, glabels, _) = build_container_rows(ACCOUNT, &graph).unwrap();
    assert!(glabels.is_empty());
    assert_eq!(find_folder(&gfolders, "INBOX").parent_id, None);
    assert_eq!(
        find_folder(&gfolders, "graph-g-work").parent_id.as_deref(),
        Some("INBOX")
    );
    assert_eq!(
        find_folder(&gfolders, "graph-g-sub").parent_id.as_deref(),
        Some("graph-g-work")
    );
    assert!(!find_folder(&gfolders, "graph-g-work").is_undeletable);

    // JMAP: user mailbox is a folder by `kind`, even with no role/system.
    let jmap = vec![
        folder(
            ProtocolKind::Jmap,
            "mb1",
            "Inbox",
            Some(FolderRole::Inbox),
            None,
        ),
        folder(ProtocolKind::Jmap, "mb2", "Projects", None, None),
    ];
    let (jfolders, _, _) = build_container_rows(ACCOUNT, &jmap).unwrap();
    let mut jids = ids(&jfolders);
    jids.sort_unstable();
    assert_eq!(jids, ["INBOX", "jmap-mb2"]);
    assert!(!find_folder(&jfolders, "jmap-mb2").is_undeletable);

    // IMAP: path-native folders, special-use carried for system folders.
    let imap = vec![
        folder(
            ProtocolKind::Imap,
            "INBOX",
            "Inbox",
            Some(FolderRole::Inbox),
            None,
        ),
        folder(
            ProtocolKind::Imap,
            "INBOX/Work",
            "Work",
            None,
            Some("INBOX"),
        ),
    ];
    let (ifolders, _, _) = build_container_rows(ACCOUNT, &imap).unwrap();
    let inbox = find_folder(&ifolders, "INBOX");
    assert_eq!(inbox.imap_folder_path.as_deref(), Some("INBOX"));
    assert_eq!(inbox.imap_special_use.as_deref(), Some("\\Inbox"));
    let work = find_folder(&ifolders, "folder-INBOX/Work");
    assert_eq!(work.imap_folder_path.as_deref(), Some("INBOX/Work"));
    assert_eq!(work.parent_id.as_deref(), Some("INBOX"));

    // Preservation semantics through the real upsert seam: a pre-seeded
    // `user_color_*` override and an `importance:*` undeletable row must
    // survive a re-sync.
    assert_preservation_semantics();
}

fn assert_preservation_semantics() {
    let mut conn = Connection::open_in_memory().unwrap();
    db::db::migrations::run_all(&conn).unwrap();

    // Pre-seed: a user color override on the very label the sync will touch,
    // and an importance:high synth row marked undeletable (never appears in
    // `containers_list`, so the sync must not clobber it).
    conn.execute(
        "INSERT INTO labels (id, account_id, name, user_color_bg, user_color_fg, is_undeletable) \
         VALUES ('Label_42', ?1, 'Work', '#abcdef', '#123456', 0)",
        rusqlite::params![ACCOUNT],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO labels (id, account_id, name, is_undeletable) \
         VALUES ('importance:high', ?1, 'High importance', 1)",
        rusqlite::params![ACCOUNT],
    )
    .unwrap();

    let (folder_rows, label_rows, _) = build_container_rows(ACCOUNT, &gmail_fixture()).unwrap();
    let tx = conn.transaction().unwrap();
    db::db::queries_extra::insert_folders_batch(&tx, &folder_rows).unwrap();
    db::db::queries_extra::upsert_labels(&tx, &label_rows).unwrap();
    tx.commit().unwrap();

    // Server color now set from the container style; user override preserved.
    let (sbg, ubg): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT server_color_bg, user_color_bg FROM labels WHERE id = 'Label_42' AND account_id = ?1",
            rusqlite::params![ACCOUNT],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(sbg.as_deref(), Some("#16a766"), "server color synced");
    assert_eq!(ubg.as_deref(), Some("#abcdef"), "user override preserved");

    // The importance synth row's undeletable flag survived the re-sync.
    let importance_undeletable: i64 = conn
        .query_row(
            "SELECT is_undeletable FROM labels WHERE id = 'importance:high' AND account_id = ?1",
            rusqlite::params![ACCOUNT],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(importance_undeletable, 1, "importance:* stays undeletable");

    // The Gmail folder rows landed in the folders table.
    let folder_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM folders WHERE account_id = ?1",
            rusqlite::params![ACCOUNT],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(folder_count, 6, "six Gmail system folders persisted");
}
