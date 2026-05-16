use rusqlite::{Transaction, params};
use types::{FolderKind, LabelKind};

use super::label_intent::finalize_provider_truth_label_membership;
use super::thread_persistence::{
    delete_thread_folder_rows, delete_thread_label_rows, insert_thread_folder_rows,
    insert_thread_label_rows,
};

pub fn replace_message_folder_rows(
    tx: &Transaction,
    account_id: &str,
    message_id: &str,
    folders: &[FolderKind],
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM message_folders WHERE account_id = ?1 AND message_id = ?2",
        params![account_id, message_id],
    )
    .map_err(|e| format!("delete message folders: {e}"))?;

    for folder in folders {
        tx.execute(
            "INSERT OR IGNORE INTO message_folders (account_id, message_id, folder_id) \
             VALUES (?1, ?2, ?3)",
            params![account_id, message_id, folder.storage_id()],
        )
        .map_err(|e| format!("insert message folder: {e}"))?;
    }

    Ok(())
}

pub fn replace_message_label_rows(
    tx: &Transaction,
    account_id: &str,
    message_id: &str,
    labels: &[LabelKind],
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM message_labels WHERE account_id = ?1 AND message_id = ?2",
        params![account_id, message_id],
    )
    .map_err(|e| format!("delete message labels: {e}"))?;

    for label in labels {
        tx.execute(
            "INSERT OR IGNORE INTO message_labels (account_id, message_id, label_id) \
             VALUES (?1, ?2, ?3)",
            params![account_id, message_id, label.storage_id()],
        )
        .map_err(|e| format!("insert message label: {e}"))?;
    }

    Ok(())
}

pub fn delete_message_membership_rows(
    tx: &Transaction,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM message_folders WHERE account_id = ?1 AND message_id = ?2",
        params![account_id, message_id],
    )
    .map_err(|e| format!("delete message folder membership: {e}"))?;
    tx.execute(
        "DELETE FROM message_labels WHERE account_id = ?1 AND message_id = ?2",
        params![account_id, message_id],
    )
    .map_err(|e| format!("delete message label membership: {e}"))?;
    Ok(())
}

pub fn recompute_thread_folders_from_messages(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    delete_thread_folder_rows(tx, account_id, thread_id)?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) \
         SELECT DISTINCT m.account_id, m.thread_id, mf.folder_id \
         FROM messages m \
         JOIN message_folders mf ON mf.account_id = m.account_id AND mf.message_id = m.id \
         WHERE m.account_id = ?1 AND m.thread_id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute thread folders from message_folders: {e}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) \
         SELECT DISTINCT m.account_id, m.thread_id, m.imap_folder \
         FROM messages m \
         WHERE m.account_id = ?1 AND m.thread_id = ?2 AND m.imap_folder IS NOT NULL",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute thread folders from imap_folder: {e}"))?;

    Ok(())
}

pub fn recompute_thread_labels_from_messages(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    delete_thread_label_rows(tx, account_id, thread_id)?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
         SELECT DISTINCT m.account_id, m.thread_id, ml.label_id \
         FROM messages m \
         JOIN message_labels ml ON ml.account_id = m.account_id AND ml.message_id = m.id \
         WHERE m.account_id = ?1 AND m.thread_id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute thread labels from message_labels: {e}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
         SELECT DISTINCT m.account_id, m.thread_id, mk.label_id \
         FROM messages m \
         JOIN message_keywords mk ON mk.account_id = m.account_id AND mk.message_id = m.id \
         WHERE m.account_id = ?1 AND m.thread_id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("recompute thread labels from message_keywords: {e}"))?;

    finalize_provider_truth_label_membership(tx, account_id, thread_id)
}

pub fn insert_full_thread_folders(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    folders: &[FolderKind],
) -> Result<(), String> {
    let folder_ids = folders.iter().map(FolderKind::storage_id).collect::<Vec<_>>();
    insert_thread_folder_rows(tx, account_id, thread_id, folder_ids.iter().map(String::as_str))
}

pub fn insert_full_thread_labels(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    labels: &[LabelKind],
) -> Result<(), String> {
    let label_ids = labels.iter().map(LabelKind::storage_id).collect::<Vec<_>>();
    insert_thread_label_rows(tx, account_id, thread_id, label_ids.iter().map(String::as_str))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use rusqlite::{Connection, params};
    use types::{FolderKind, LabelKind, SystemFolderId};

    use super::{
        recompute_thread_folders_from_messages, recompute_thread_labels_from_messages,
        replace_message_folder_rows, replace_message_label_rows,
    };

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::migrations::run_all(&conn).unwrap();
        conn.execute(
            "INSERT INTO accounts (id, email, provider) VALUES ('acc', 'a@example.com', 'graph')",
            [],
        )
        .unwrap();
        for (id, name) in [
            ("INBOX", "Inbox"),
            ("archive", "Archive"),
            ("graph-projects", "Projects"),
        ] {
            conn.execute(
                "INSERT INTO folders (id, account_id, name) VALUES (?1, 'acc', ?2)",
                params![id, name],
            )
            .unwrap();
        }
        for (id, name) in [("cat:Blue", "Blue"), ("cat:Red", "Red")] {
            conn.execute(
                "INSERT INTO labels (id, account_id, name, server_color_bg, server_color_fg) \
                 VALUES (?1, 'acc', ?2, '#123456', '#ffffff')",
                params![id, name],
            )
            .unwrap();
        }
        conn
    }

    fn insert_thread(conn: &Connection, thread_id: &str) {
        conn.execute(
            "INSERT INTO threads (id, account_id, subject, snippet, last_message_at, message_count) \
             VALUES (?1, 'acc', 'subject', 'snippet', 1, 1)",
            params![thread_id],
        )
        .unwrap();
    }

    fn insert_message(conn: &Connection, message_id: &str, thread_id: &str, date: i64) {
        conn.execute(
            "INSERT INTO messages (id, account_id, thread_id, date, subject, snippet) \
             VALUES (?1, 'acc', ?2, ?3, 'subject', 'snippet')",
            params![message_id, thread_id, date],
        )
        .unwrap();
    }

    fn thread_folders(conn: &Connection, thread_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id FROM thread_folders \
                 WHERE account_id = 'acc' AND thread_id = ?1 ORDER BY folder_id",
            )
            .unwrap();
        stmt.query_map(params![thread_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn thread_labels(conn: &Connection, thread_id: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT label_id FROM thread_labels \
                 WHERE account_id = 'acc' AND thread_id = ?1 ORDER BY label_id",
            )
            .unwrap();
        stmt.query_map(params![thread_id], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn replace_membership(
        conn: &mut Connection,
        message_id: &str,
        folders: &[FolderKind],
        labels: &[LabelKind],
    ) {
        let tx = conn.unchecked_transaction().unwrap();
        replace_message_folder_rows(&tx, "acc", message_id, folders).unwrap();
        replace_message_label_rows(&tx, "acc", message_id, labels).unwrap();
        tx.commit().unwrap();
    }

    fn recompute(conn: &mut Connection, thread_id: &str) {
        let tx = conn.unchecked_transaction().unwrap();
        recompute_thread_folders_from_messages(&tx, "acc", thread_id).unwrap();
        recompute_thread_labels_from_messages(&tx, "acc", thread_id).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn recompute_removes_stale_per_message_membership() {
        let mut conn = setup_conn();
        insert_thread(&conn, "t1");
        insert_message(&conn, "m1", "t1", 1);
        insert_message(&conn, "m2", "t1", 2);

        replace_membership(
            &mut conn,
            "m1",
            &[FolderKind::System(SystemFolderId::Inbox)],
            &[LabelKind::graph_category("Blue").unwrap()],
        );
        replace_membership(
            &mut conn,
            "m2",
            &[FolderKind::System(SystemFolderId::Archive)],
            &[LabelKind::graph_category("Red").unwrap()],
        );
        recompute(&mut conn, "t1");
        assert_eq!(thread_folders(&conn, "t1"), vec!["INBOX", "archive"]);
        assert_eq!(thread_labels(&conn, "t1"), vec!["cat:Blue", "cat:Red"]);

        replace_membership(
            &mut conn,
            "m1",
            &[FolderKind::System(SystemFolderId::Archive)],
            &[],
        );
        recompute(&mut conn, "t1");

        assert_eq!(thread_folders(&conn, "t1"), vec!["archive"]);
        assert_eq!(thread_labels(&conn, "t1"), vec!["cat:Red"]);
    }

    #[test]
    fn delete_message_recomputes_membership_from_remaining_messages() {
        let mut conn = setup_conn();
        insert_thread(&conn, "t1");
        insert_message(&conn, "m1", "t1", 1);
        insert_message(&conn, "m2", "t1", 2);
        replace_membership(
            &mut conn,
            "m1",
            &[FolderKind::System(SystemFolderId::Inbox)],
            &[LabelKind::graph_category("Blue").unwrap()],
        );
        replace_membership(
            &mut conn,
            "m2",
            &[FolderKind::System(SystemFolderId::Archive)],
            &[LabelKind::graph_category("Red").unwrap()],
        );
        recompute(&mut conn, "t1");

        let tx = conn.unchecked_transaction().unwrap();
        super::super::thread_persistence::delete_messages_and_cleanup_threads(&tx, "acc", &["m1"])
            .unwrap();
        tx.commit().unwrap();

        assert_eq!(thread_folders(&conn, "t1"), vec!["archive"]);
        assert_eq!(thread_labels(&conn, "t1"), vec!["cat:Red"]);
    }

    #[test]
    fn reassign_message_recomputes_old_and_new_threads() {
        let mut conn = setup_conn();
        insert_thread(&conn, "old");
        insert_thread(&conn, "new");
        insert_message(&conn, "m1", "old", 1);
        insert_message(&conn, "m2", "old", 2);
        replace_membership(
            &mut conn,
            "m1",
            &[FolderKind::System(SystemFolderId::Inbox)],
            &[LabelKind::graph_category("Blue").unwrap()],
        );
        replace_membership(
            &mut conn,
            "m2",
            &[FolderKind::System(SystemFolderId::Archive)],
            &[LabelKind::graph_category("Red").unwrap()],
        );
        recompute(&mut conn, "old");

        let tx = conn.unchecked_transaction().unwrap();
        super::super::thread_persistence::reassign_messages_and_repair_threads(
            &tx,
            "acc",
            "new",
            &["m1"],
            &[],
        )
        .unwrap();
        tx.commit().unwrap();

        assert_eq!(thread_folders(&conn, "old"), vec!["archive"]);
        assert_eq!(thread_labels(&conn, "old"), vec!["cat:Red"]);
        assert_eq!(thread_folders(&conn, "new"), vec!["INBOX"]);
        assert_eq!(thread_labels(&conn, "new"), vec!["cat:Blue"]);
    }
}
