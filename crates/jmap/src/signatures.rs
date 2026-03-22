use sha2::{Digest, Sha256};

use jmap_client::identity::{IdentityGet, IdentitySet};

use ratatoskr_db::db::DbState;

use super::client::JmapClient;

// ---------------------------------------------------------------------------
// Sync: pull JMAP identities → signatures table
// ---------------------------------------------------------------------------

/// Fetch all JMAP identities and upsert their signatures into the local DB.
///
/// Each identity's `htmlSignature` / `textSignature` becomes one row in the
/// `signatures` table, keyed by `(account_id, server_id)`.  The first identity
/// is marked as the default when no default exists yet.
pub async fn sync_jmap_identity_signatures(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
) -> Result<usize, String> {
    let identities = fetch_all_identities(client).await?;

    let aid = account_id.to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs().cast_signed();

    // Collect data before moving into the closure.
    let rows: Vec<IdentityRow> = identities
        .into_iter()
        .filter_map(|mut ident| {
            let id = ident.id.take()?;
            let name = ident.name.take().unwrap_or_default();
            let html = ident.html_signature.take().unwrap_or_default();
            let text = ident.text_signature.take().unwrap_or_default();
            let html_hash = sha256_hex(&html);
            Some(IdentityRow {
                server_id: id,
                name,
                body_html: html,
                body_text: text,
                server_html_hash: html_hash,
            })
        })
        .collect();

    let count = rows.len();

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        // Check whether the account already has a default signature.
        let has_default: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM signatures WHERE account_id = ?1 AND is_default = 1) AS has_default",
                rusqlite::params![aid],
                |row| row.get("has_default"),
            )
            .unwrap_or(false);

        for (idx, row) in rows.iter().enumerate() {
            let is_default = if !has_default && idx == 0 { 1i64 } else { 0i64 };

            tx.execute(
                "INSERT INTO signatures (id, account_id, name, body_html, body_text, is_default, server_id, source, last_synced_at, server_html_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'jmap_sync', ?8, ?9)
                 ON CONFLICT(account_id, server_id) DO UPDATE SET
                   name = excluded.name,
                   body_html = excluded.body_html,
                   body_text = excluded.body_text,
                   last_synced_at = excluded.last_synced_at,
                   server_html_hash = excluded.server_html_hash",
                rusqlite::params![
                    uuid::Uuid::new_v4().to_string(),
                    aid,
                    row.name,
                    row.body_html,
                    row.body_text,
                    is_default,
                    row.server_id,
                    now,
                    row.server_html_hash,
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(count)
    })
    .await
}

// ---------------------------------------------------------------------------
// Push: local signature edits → Identity/set
// ---------------------------------------------------------------------------

/// Push a local signature's HTML and text content to the corresponding JMAP
/// identity.  `identity_id` is the JMAP server-side identity ID (stored as
/// `server_id` in the `signatures` table).
pub async fn push_signature_to_jmap(
    client: &JmapClient,
    identity_id: &str,
    html: &str,
    text: &str,
) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let mut set = IdentitySet::new(&account_id);
    set.update(identity_id)
        .html_signature(html)
        .text_signature(text);
    let handle = request
        .call(set)
        .map_err(|e| format!("Identity/set: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Identity/set: {e}"))?;

    // Check for per-item errors.
    response
        .get(&handle)
        .map_err(|e| format!("Identity/set: {e}"))?
        .updated(identity_id)
        .map_err(|e| format!("Identity/set update {identity_id}: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct IdentityRow {
    server_id: String,
    name: String,
    body_html: String,
    body_text: String,
    server_html_hash: String,
}

/// Fetch all JMAP identities via `Identity/get` (no ID filter = all).
async fn fetch_all_identities(
    client: &JmapClient,
) -> Result<Vec<jmap_client::identity::Identity>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let get = IdentityGet::new(&account_id);
    let handle = request
        .call(get)
        .map_err(|e| format!("Identity/get: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Identity/get: {e}"))?;

    let mut get_response = response
        .get(&handle)
        .map_err(|e| format!("Identity/get: {e}"))?;

    Ok(get_response.take_list())
}

/// SHA-256 hex digest of a string.
fn sha256_hex(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    hex_encode(hash)
}

/// Minimal hex encoding to avoid adding a dependency.
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes
        .as_ref()
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}
