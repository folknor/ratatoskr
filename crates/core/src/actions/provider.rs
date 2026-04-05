use crate::db::DbState;
use common::ops::ProviderOps;

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
    let aid = account_id.to_string();
    let provider = db
        .with_conn(move |conn| {
            crate::db::queries_extra::contacts::get_account_provider_sync(conn, &aid)
        })
        .await?;

    match provider.as_str() {
        "gmail_api" => {
            let client =
                gmail::client::GmailClient::from_account(db, account_id, encryption_key).await?;
            Ok(Box::new(gmail::ops::GmailOps::new(client)))
        }
        "graph" => {
            let client =
                graph::client::GraphClient::from_account(db, account_id, encryption_key).await?;
            Ok(Box::new(graph::ops::GraphOps::new(client)))
        }
        "jmap" => {
            let client =
                jmap::client::JmapClient::from_account(db, account_id, &encryption_key).await?;
            Ok(Box::new(jmap::ops::JmapOps::new(client)))
        }
        "imap" => Ok(Box::new(imap::ops::ImapOps::new(encryption_key))),
        other => Err(format!("Unknown provider: {other}")),
    }
}
