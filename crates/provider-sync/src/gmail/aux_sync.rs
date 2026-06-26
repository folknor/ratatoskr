use std::collections::HashMap;

use sha2::{Digest, Sha256};

use super::client::GmailClient;
use super::types::{GmailLabel, GmailSendAs};
use common::types::{FolderKind, MailProviderKind};
use db::db::queries_extra::{FolderWriteRow, LabelWriteRow, insert_folders_batch, upsert_labels};
use db::db::{ReadConn, ReadDbState, WriteConn, WriteTarget};
use service_state::WriteDbState;

pub async fn sync_gmail_label_folder_map(
    client: &GmailClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<HashMap<String, FolderKind>, String> {
    let labels = client.list_labels(read_db).await?;
    let aid = account_id.to_string();
    let folder_map = write_db
        .with_write(move |conn| persist_folders_and_labels(conn, &aid, &labels))
        .await?;
    Ok(folder_map)
}

fn persist_folders_and_labels(
    conn: &WriteConn<'_>,
    account_id: &str,
    labels: &[GmailLabel],
) -> Result<HashMap<String, FolderKind>, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin label tx: {e}"))?;

    let mut folder_rows = Vec::new();
    let mut label_rows = Vec::new();
    let mut folder_map = HashMap::new();

    for label in labels
        .iter()
        .filter(|label| !common::folder_roles::is_message_state_label_id(&label.id))
    {
        let label_type = label.label_type.as_deref().unwrap_or("user");
        if label_type == "system" {
            let folder = FolderKind::parse(&label.id, MailProviderKind::Gmail)?;
            folder_map.insert(label.id.clone(), folder);
            folder_rows.push(FolderWriteRow {
                id: label.id.clone(),
                account_id: account_id.to_string(),
                name: label.name.clone(),
                visible: None,
                sort_order: None,
                imap_folder_path: None,
                imap_special_use: None,
                namespace_type: None,
                parent_id: None,
                right_read: None,
                right_add: None,
                right_remove: None,
                right_set_seen: None,
                right_set_keywords: None,
                right_create_child: None,
                right_rename: None,
                right_delete: None,
                right_submit: None,
                is_subscribed: None,
                is_undeletable: true,
            });
        } else {
            label_rows.push(LabelWriteRow {
                id: label.id.clone(),
                account_id: account_id.to_string(),
                name: label.name.clone(),
                visible: None,
                sort_order: None,
                server_color_bg: label.color.as_ref().map(|c| c.background_color.clone()),
                server_color_fg: label.color.as_ref().map(|c| c.text_color.clone()),
                user_color_bg: None,
                user_color_fg: None,
                is_undeletable: false,
            });
        }
    }

    insert_folders_batch(&tx, &folder_rows)?;
    upsert_labels(&tx, &label_rows)?;
    tx.commit()
        .map_err(|e| format!("commit folders + labels: {e}"))?;
    Ok(folder_map)
}

#[allow(clippy::too_many_arguments)]
pub async fn run_gmail_auxiliary_sync(
    client: &GmailClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
    initial_sync_completed_before_run: bool,
) {
    if let Err(error) = sync_gmail_signatures(client, account_id, read_db, write_db).await {
        log::warn!("Gmail signature sync failed for account {account_id}: {error}");
    }

    let should_sync_contacts = if initial_sync_completed_before_run {
        match sync::state::increment_gmail_sync_cycle(&write_db.writer_pool(), account_id).await {
            Ok(cycle) => cycle.is_multiple_of(20),
            Err(error) => {
                log::warn!("Gmail sync-cycle increment failed for account {account_id}: {error}");
                false
            }
        }
    } else {
        true
    };

    if should_sync_contacts {
        if let Err(error) = super::contacts::sync_google_contacts(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            log::warn!("Google contacts sync failed for account {account_id}: {error}");
        }
        let writer = write_db.clone();
        if let Err(error) = super::contacts::sync_google_other_contacts(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
            move |write| {
                let writer = writer.clone();
                async move {
                    writer
                        .with_write(move |conn| {
                            let tx = conn
                                .transaction()
                                .map_err(|e| format!("begin google other contacts tx: {e}"))?;
                            super::contacts::persist_google_other_contacts_write(&tx, write)?;
                            tx.commit()
                                .map_err(|e| format!("commit google other contacts tx: {e}"))?;
                            Ok(())
                        })
                        .await
                }
            },
        )
        .await
        {
            log::warn!("Google otherContacts sync failed for account {account_id}: {error}");
        }
    }
}

async fn sync_gmail_signatures(
    client: &GmailClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<(), String> {
    let aliases = client.list_send_as(read_db).await?;

    let aid = account_id.to_string();
    let locals: Vec<LocalSignature> = read_db
        .with_read(move |conn| read_local_signatures(conn, &aid))
        .await?;

    let local_map: HashMap<&str, &LocalSignature> =
        locals.iter().map(|l| (l.server_id.as_str(), l)).collect();

    let now = chrono::Utc::now().timestamp();
    let mut push_queue: Vec<(String, String)> = Vec::new();

    for (i, alias) in aliases.iter().enumerate() {
        let server_id = &alias.send_as_email;
        let server_html = alias.signature.as_deref().unwrap_or("");
        let server_hash_now = html_hash(server_html);
        let local = local_map.get(server_id.as_str()).copied();

        let action = determine_sync_action(local, server_html, &server_hash_now);
        match action {
            SigSyncAction::NoOp => {}
            SigSyncAction::PullFromServer | SigSyncAction::ConflictServerWins => {
                if matches!(action, SigSyncAction::ConflictServerWins) {
                    log::warn!(
                        "Signature conflict for {server_id} - both local and server changed. \
                         Preferring server version."
                    );
                }
                let name = build_sig_name(alias, server_id);
                let is_default = i64::from(alias.is_default.unwrap_or(false));
                let id = format!("gmail_sig_{account_id}_{server_id}");
                let aid = account_id.to_string();
                let sid = server_id.clone();
                let html = server_html.to_string();
                let hash = server_hash_now.clone();
                #[allow(clippy::cast_possible_wrap)]
                let sort = i as i64;

                write_db
                    .with_write(move |conn| {
                        upsert_signature_from_server(
                            conn, &id, &aid, &name, &html, is_default, sort, &sid, &hash, now,
                        )
                    })
                    .await?;
            }
            SigSyncAction::PushToServer => {
                if let Some(loc) = local {
                    push_queue.push((server_id.clone(), loc.body_html.clone()));
                    let local_hash = html_hash(&loc.body_html);
                    let lid = loc.id.clone();
                    write_db
                        .with_write(move |conn| {
                            conn.execute(
                                "UPDATE signatures SET server_html_hash = ?1, last_synced_at = ?2 \
                                 WHERE id = ?3",
                                rusqlite::params![local_hash, now, lid],
                            )
                            .map_err(|e| format!("update sig hash after push: {e}"))?;
                            Ok(())
                        })
                        .await?;
                }
            }
        }
    }

    for (send_as_email, html) in &push_queue {
        if let Err(error) = client
            .update_send_as_signature(send_as_email, html, read_db)
            .await
        {
            log::error!("Failed to push signature for {send_as_email}: {error}");
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct LocalSignature {
    id: String,
    server_id: String,
    body_html: String,
    server_html_hash: Option<String>,
    name: String,
    is_default: bool,
    sort_order: i64,
}

enum SigSyncAction {
    PullFromServer,
    PushToServer,
    ConflictServerWins,
    NoOp,
}

fn determine_sync_action(
    local: Option<&LocalSignature>,
    server_html: &str,
    server_hash_now: &str,
) -> SigSyncAction {
    let Some(loc) = local else {
        if server_html.is_empty() {
            return SigSyncAction::NoOp;
        }
        return SigSyncAction::PullFromServer;
    };

    let stored_server_hash = loc.server_html_hash.as_deref().unwrap_or("");
    let local_hash = html_hash(&loc.body_html);

    match (
        server_hash_now != stored_server_hash,
        local_hash != stored_server_hash,
    ) {
        (false, false) => SigSyncAction::NoOp,
        (true, false) => SigSyncAction::PullFromServer,
        (false, true) => SigSyncAction::PushToServer,
        (true, true) => SigSyncAction::ConflictServerWins,
    }
}

#[allow(clippy::too_many_arguments)]
fn upsert_signature_from_server(
    conn: &impl WriteTarget,
    id: &str,
    account_id: &str,
    name: &str,
    body_html: &str,
    is_default: i64,
    sort_order: i64,
    server_id: &str,
    server_html_hash: &str,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO signatures \
         (id, account_id, name, body_html, is_default, sort_order, \
          server_id, source, server_html_hash, last_synced_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_sync', ?8, ?9) \
         ON CONFLICT(account_id, server_id) DO UPDATE SET \
           name = excluded.name, \
           body_html = excluded.body_html, \
           is_default = excluded.is_default, \
           sort_order = excluded.sort_order, \
           server_html_hash = excluded.server_html_hash, \
           last_synced_at = excluded.last_synced_at",
        rusqlite::params![
            id,
            account_id,
            name,
            body_html,
            is_default,
            sort_order,
            server_id,
            server_html_hash,
            now,
        ],
    )
    .map_err(|e| format!("upsert gmail signature: {e}"))?;
    Ok(())
}

fn read_local_signatures(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<LocalSignature>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, server_id, body_html, server_html_hash, name, is_default, sort_order \
             FROM signatures WHERE account_id = ?1 AND server_id IS NOT NULL",
        )
        .map_err(|e| format!("prepare read_local_signatures: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(LocalSignature {
                id: row.get("id")?,
                server_id: row.get("server_id")?,
                body_html: row.get("body_html")?,
                server_html_hash: row.get("server_html_hash")?,
                name: row.get("name")?,
                is_default: row.get::<_, i64>("is_default")? != 0,
                sort_order: row.get("sort_order")?,
            })
        })
        .map_err(|e| format!("query local signatures: {e}"))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| format!("read signature row: {e}"))?);
    }
    Ok(result)
}

fn build_sig_name(alias: &GmailSendAs, server_id: &str) -> String {
    alias
        .display_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .map_or_else(
            || server_id.to_string(),
            |name| format!("{name} ({server_id})"),
        )
}

fn html_hash(html: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    hex_encode(hasher.finalize())
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
