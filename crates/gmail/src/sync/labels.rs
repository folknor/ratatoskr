use sha2::{Digest, Sha256};

use super::super::client::GmailClient;
use super::super::types::{GmailLabel, GmailSendAs};
use super::SyncCtx;
use db::db::DbState;
use db::db::queries_extra::{LabelWriteRow, upsert_labels};

// ---------------------------------------------------------------------------
// Label sync
// ---------------------------------------------------------------------------

pub(super) async fn sync_labels(ctx: &SyncCtx<'_>) -> Result<(), String> {
    let labels = ctx.client.list_labels(ctx.db).await?;

    let aid = ctx.account_id.to_string();
    ctx.db
        .with_conn(move |conn| persist_labels(conn, &aid, &labels))
        .await
}

fn persist_labels(
    conn: &rusqlite::Connection,
    account_id: &str,
    labels: &[GmailLabel],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin label tx: {e}"))?;

    let rows: Vec<LabelWriteRow> = labels
        .iter()
        .map(|label| {
            let color_bg = label.color.as_ref().map(|c| c.background_color.clone());
            let color_fg = label.color.as_ref().map(|c| c.text_color.clone());
            let label_type = label.label_type.as_deref().unwrap_or("user");
            let label_kind = if label_type == "system" {
                "container"
            } else {
                "tag"
            };

            LabelWriteRow {
                id: label.id.clone(),
                account_id: account_id.to_string(),
                name: label.name.clone(),
                label_type: label_type.to_string(),
                label_kind: label_kind.to_string(),
                color_bg,
                color_fg,
                sort_order: None,
                imap_folder_path: None,
                imap_special_use: None,
                parent_label_id: None,
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
            }
        })
        .collect();

    upsert_labels(&tx, &rows)?;
    tx.commit().map_err(|e| format!("commit labels: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Signature sync (bidirectional)
// ---------------------------------------------------------------------------

/// Local signature row read from the `signatures` table for diff comparison.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields used for future sync enhancements (rename, default toggle)
struct LocalSignature {
    id: String,
    server_id: String,
    body_html: String,
    server_html_hash: Option<String>,
    name: String,
    is_default: bool,
    sort_order: i64,
}

/// Action determined by comparing local and server state.
enum SigSyncAction {
    /// Server changed, local did not (or new) → pull server HTML to local.
    PullFromServer,
    /// Local changed, server did not → push local HTML to server.
    PushToServer,
    /// Both changed → prefer server, log conflict warning.
    ConflictServerWins,
    /// No change on either side → skip.
    NoOp,
}

/// Bidirectional signature sync.
///
/// For each Gmail sendAs alias:
/// 1. Fetch current server signatures via `list_send_as()`.
/// 2. Compare `server_html_hash` (stored locally from last sync) against the
///    current server HTML hash to detect server-side changes.
/// 3. Compare local `body_html` hash against `server_html_hash` to detect
///    local edits.
/// 4. Resolve:
///    - Server changed, local didn't → update local (`body_html`, hash, timestamp).
///    - Local changed, server didn't → push to Gmail API.
///    - Both changed → prefer server (log a conflict warning).
///    - Neither changed → no-op.
pub(super) async fn sync_signatures(ctx: &SyncCtx<'_>) -> Result<(), String> {
    let aliases = ctx.client.list_send_as(ctx.db).await?;

    // Read existing local signatures for this account
    let aid = ctx.account_id.to_string();
    let locals: Vec<LocalSignature> = ctx
        .db
        .with_conn(move |conn| read_local_signatures(conn, &aid))
        .await?;

    // Build a lookup: server_id → LocalSignature
    let local_map: std::collections::HashMap<&str, &LocalSignature> =
        locals.iter().map(|l| (l.server_id.as_str(), l)).collect();

    let now = chrono::Utc::now().timestamp();

    // Collect push-to-server actions to execute after the DB pass
    let mut push_queue: Vec<(String, String)> = Vec::new();

    for (i, alias) in aliases.iter().enumerate() {
        let server_id = &alias.send_as_email;
        let server_html = alias.signature.as_deref().unwrap_or("");

        // Compute hash of current server HTML
        let server_hash_now = html_hash(server_html);

        let local = local_map.get(server_id.as_str()).copied();

        let action = determine_sync_action(local, server_html, &server_hash_now);

        match action {
            SigSyncAction::NoOp => {}

            SigSyncAction::PullFromServer | SigSyncAction::ConflictServerWins => {
                if matches!(action, SigSyncAction::ConflictServerWins) {
                    log::warn!(
                        "Signature conflict for {server_id} — both local and server changed. \
                         Preferring server version."
                    );
                }

                let name = build_sig_name(alias, server_id);
                let is_default = i64::from(alias.is_default.unwrap_or(false));
                let id = format!(
                    "gmail_sig_{account_id}_{server_id}",
                    account_id = ctx.account_id
                );
                let aid = ctx.account_id.to_string();
                let sid = server_id.clone();
                let html = server_html.to_string();
                let hash = server_hash_now.clone();
                #[allow(clippy::cast_possible_wrap)]
                let sort = i as i64;

                ctx.db
                    .with_conn(move |conn| {
                        upsert_signature_from_server(
                            conn, &id, &aid, &name, &html, is_default, sort, &sid, &hash, now,
                        )
                    })
                    .await?;
            }

            SigSyncAction::PushToServer => {
                if let Some(loc) = local {
                    push_queue.push((server_id.clone(), loc.body_html.clone()));

                    // After push, update the stored hash to match what we just pushed
                    let local_hash = html_hash(&loc.body_html);
                    let lid = loc.id.clone();
                    ctx.db
                        .with_conn(move |conn| {
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

    // Execute pushes to Gmail API
    for (send_as_email, html) in &push_queue {
        if let Err(e) = push_signature_to_gmail(ctx.client, send_as_email, html, ctx.db).await {
            log::error!("Failed to push signature for {send_as_email}: {e}");
        }
    }

    Ok(())
}

/// Determine what sync action to take for a single signature.
fn determine_sync_action(
    local: Option<&LocalSignature>,
    server_html: &str,
    server_hash_now: &str,
) -> SigSyncAction {
    let Some(loc) = local else {
        // No local row yet — if server has content, pull it
        if server_html.is_empty() {
            return SigSyncAction::NoOp;
        }
        return SigSyncAction::PullFromServer;
    };

    let stored_server_hash = loc.server_html_hash.as_deref().unwrap_or("");
    let local_hash = html_hash(&loc.body_html);

    let server_changed = server_hash_now != stored_server_hash;
    let local_changed = local_hash != stored_server_hash;

    match (server_changed, local_changed) {
        (false, false) => SigSyncAction::NoOp,
        (true, false) => SigSyncAction::PullFromServer,
        (false, true) => SigSyncAction::PushToServer,
        (true, true) => SigSyncAction::ConflictServerWins,
    }
}

/// Push a local signature to Gmail via the sendAs API.
async fn push_signature_to_gmail(
    client: &GmailClient,
    send_as_email: &str,
    html: &str,
    db: &DbState,
) -> Result<(), String> {
    client
        .update_send_as_signature(send_as_email, html, db)
        .await?;
    log::info!("Pushed signature update to Gmail for {send_as_email}");
    Ok(())
}

/// Upsert a signature row from server data (pull path).
#[allow(clippy::too_many_arguments)]
fn upsert_signature_from_server(
    conn: &rusqlite::Connection,
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

/// Read all local signatures for an account that have a `server_id` (i.e. came
/// from Gmail sync or were linked to a sendAs alias).
fn read_local_signatures(
    conn: &rusqlite::Connection,
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

/// Build a human-readable display name for a signature.
fn build_sig_name(alias: &GmailSendAs, server_id: &str) -> String {
    alias
        .display_name
        .as_deref()
        .filter(|n| !n.is_empty())
        .map_or_else(|| server_id.to_string(), |n| format!("{n} ({server_id})"))
}

/// SHA-256 hash of HTML content, hex-encoded.
fn html_hash(html: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(html.as_bytes());
    hex_encode(hasher.finalize())
}

/// Minimal hex encoding (same pattern as bimi.rs).
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
