use crate::db::DbState;
use ratatoskr_provider_utils::ops::ProviderOps;

/// Create a provider ops instance for the given account.
///
/// Reads the provider type from the database, decrypts credentials with
/// the encryption key, and constructs the appropriate provider client.
///
/// This is the single point of provider resolution. The app crate should
/// not construct providers directly.
pub async fn create_provider(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderOps>, String> {
    let conn = db.conn();
    let aid = account_id.to_string();
    let provider = tokio::task::spawn_blocking(move || {
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        conn.query_row(
            "SELECT provider FROM accounts WHERE id = ?1",
            rusqlite::params![aid],
            |row| row.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))??;

    match provider.as_str() {
        "gmail_api" => {
            let client = ratatoskr_gmail::client::GmailClient::from_account(
                db,
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(ratatoskr_gmail::ops::GmailOps::new(client)))
        }
        "graph" => {
            let client = ratatoskr_graph::client::GraphClient::from_account(
                db,
                account_id,
                encryption_key,
            )
            .await?;
            Ok(Box::new(ratatoskr_graph::ops::GraphOps::new(client)))
        }
        "jmap" => {
            let client = ratatoskr_jmap::client::JmapClient::from_account(
                db,
                account_id,
                &encryption_key,
            )
            .await?;
            Ok(Box::new(ratatoskr_jmap::ops::JmapOps::new(client)))
        }
        "imap" => Ok(Box::new(ratatoskr_imap::ops::ImapOps::new(
            encryption_key,
        ))),
        other => Err(format!("Unknown provider: {other}")),
    }
}
