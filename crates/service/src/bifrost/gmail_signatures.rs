use std::collections::HashMap;

use sha2::{Digest, Sha256};

use db::db::{ReadConn, ReadDbState, WriteTarget};
use service_state::WriteDbState;

use super::ResidentEngine;
use super::settings::{AccountSettingsSurface, IdentityId, IdentityPatch};

pub(crate) async fn sync_gmail_signatures(
    resident: ResidentEngine,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<(), String> {
    let settings = AccountSettingsSurface::new(resident);
    let identities = settings
        .identities(account_id)
        .await
        .map_err(|e| e.to_string())?;

    let aid = account_id.to_string();
    let locals: Vec<LocalSignature> = read_db
        .with_read(move |conn| read_local_signatures(conn, &aid))
        .await?;
    let local_map: HashMap<&str, &LocalSignature> = locals
        .iter()
        .map(|local| (local.server_id.as_str(), local))
        .collect();

    let now = jiff::Timestamp::now().as_second();
    let mut push_queue: Vec<(IdentityId, String, String)> = Vec::new();

    for (index, identity) in identities.into_iter().enumerate() {
        let identity_id = identity.id;
        let server_id = identity.address;
        let server_html = identity.signature_html.unwrap_or_default();
        let name = build_sig_name(&identity.name, &server_id);
        let is_default = i64::from(identity.is_default);
        let server_hash_now = html_hash(&server_html);
        let local = local_map.get(server_id.as_str()).copied();

        let action = determine_sync_action(local, &server_html, &server_hash_now);
        match action {
            SigSyncAction::NoOp => {}
            SigSyncAction::PullFromServer | SigSyncAction::ConflictServerWins => {
                if matches!(action, SigSyncAction::ConflictServerWins) {
                    log::warn!(
                        "Signature conflict for {server_id} - both local and server changed. Preferring server version."
                    );
                }
                let id = format!("gmail_sig_{account_id}_{server_id}");
                let aid = account_id.to_string();
                let sid = server_id.clone();
                let html = server_html.clone();
                let hash = server_hash_now.clone();
                #[allow(clippy::cast_possible_wrap)]
                let sort = index as i64;

                write_db
                    .with_write(move |conn| {
                        upsert_signature_from_server(
                            conn, &id, &aid, &name, &html, is_default, sort, &sid, &hash, now,
                        )
                    })
                    .await?;
            }
            SigSyncAction::PushToServer => {
                if let Some(local) = local {
                    push_queue.push((identity_id, server_id.clone(), local.body_html.clone()));
                    let local_hash = html_hash(&local.body_html);
                    let local_id = local.id.clone();
                    write_db
                        .with_write(move |conn| {
                            conn.execute(
                                "UPDATE signatures SET server_html_hash = ?1, last_synced_at = ?2 WHERE id = ?3",
                                rusqlite::params![local_hash, now, local_id],
                            )
                            .map_err(|e| format!("update sig hash after push: {e}"))?;
                            Ok(())
                        })
                        .await?;
                }
            }
        }
    }

    for (identity_id, address, html) in push_queue {
        let mut patch = IdentityPatch::default();
        patch.signature_html = Some(Some(html));
        if let Err(error) = settings
            .identity_update(account_id, identity_id, patch)
            .await
        {
            log::error!("Failed to push signature for {address}: {error}");
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
    let Some(local) = local else {
        if server_html.is_empty() {
            return SigSyncAction::NoOp;
        }
        return SigSyncAction::PullFromServer;
    };

    let stored_server_hash = local.server_html_hash.as_deref().unwrap_or("");
    let local_hash = html_hash(&local.body_html);
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
        "INSERT INTO signatures (id, account_id, name, body_html, is_default, sort_order, server_id, source, server_html_hash, last_synced_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'gmail_sync', ?8, ?9) ON CONFLICT(account_id, server_id) DO UPDATE SET name = excluded.name, body_html = excluded.body_html, is_default = excluded.is_default, sort_order = excluded.sort_order, server_html_hash = excluded.server_html_hash, last_synced_at = excluded.last_synced_at",
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
    let mut statement = conn
        .prepare(
            "SELECT id, server_id, body_html, server_html_hash, name, is_default, sort_order FROM signatures WHERE account_id = ?1 AND server_id IS NOT NULL",
        )
        .map_err(|e| format!("prepare read_local_signatures: {e}"))?;
    let rows = statement
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
    rows.map(|row| row.map_err(|e| format!("read signature row: {e}")))
        .collect()
}

fn build_sig_name(name: &str, server_id: &str) -> String {
    (!name.is_empty()).then_some(name).map_or_else(
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

    bytes
        .as_ref()
        .iter()
        .fold(String::new(), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}
