use db::db::ReadDbState;
use provider_sync::ProviderSyncOps;

/// Create a provider ops instance for the given account.
///
/// Reads the provider type from the database, decrypts credentials with
/// the encryption key, and constructs the appropriate provider client.
///
/// Returns `Box<dyn ProviderSyncOps>` (Phase 6d-B). The trait inherits
/// `ProviderOps` so a single trait object covers both the action and
/// sync surfaces - action callers continue to call `provider.archive(...)`
/// etc. directly via supertrait method resolution; the sync dispatcher
/// calls `provider.sync_initial(...)` / `sync_delta(...)`. The action
/// service is the single point of provider resolution; the app crate
/// must not construct providers directly.
pub async fn create_provider(
    db: &ReadDbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<Box<dyn ProviderSyncOps>, String> {
    let aid = account_id.to_string();
    let provider = db
        .with_conn(move |conn| {
            db::db::queries_extra::contacts::get_account_provider_sync(conn, &aid)
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
