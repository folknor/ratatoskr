use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::gmail::ops::GmailOps;
use crate::jmap::client::JmapState;
use crate::jmap::ops::JmapOps;

use super::ops::ProviderOps;

/// Look up the provider type for an account from the database.
pub async fn get_provider_type(db: &DbState, account_id: &str) -> Result<String, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT provider FROM accounts WHERE id = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_row([&aid], |row| row.get::<_, String>(0))
            .map_err(|e| format!("No account found for {aid}: {e}"))
    })
    .await
}

/// Resolve account → provider → `Box<dyn ProviderOps>`.
pub async fn get_ops(
    provider: &str,
    account_id: &str,
    gmail: &GmailState,
    jmap: &JmapState,
) -> Result<Box<dyn ProviderOps>, String> {
    match provider {
        "gmail_api" => {
            let client = gmail.get(account_id).await?;
            Ok(Box::new(GmailOps { client }))
        }
        "jmap" => {
            let client = jmap.get(account_id).await?;
            Ok(Box::new(JmapOps { client }))
        }
        "imap" => Err("IMAP uses TS provider path".into()),
        other => Err(format!("Unknown provider: {other}")),
    }
}
