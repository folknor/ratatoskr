use std::collections::HashMap;
use std::sync::Arc;

use jmap_client::client::Client;
use tokio::sync::RwLock;

use crate::db::DbState;
use crate::provider::crypto;

/// Per-account JMAP client.
///
/// For Basic auth the client is fully immutable after construction —
/// no token refresh, no credential mutation.
#[derive(Clone)]
pub struct JmapClient {
    inner: Arc<Client>,
}

impl JmapClient {
    /// Direct access to the underlying `jmap-client` Client.
    pub fn inner(&self) -> &Client {
        &self.inner
    }

    /// Create a JMAP client from a DB account record.
    ///
    /// Reads `jmap_url`, email, and encrypted password from the database,
    /// connects using Basic auth, and performs session discovery.
    pub async fn from_account(
        db: &DbState,
        account_id: &str,
        encryption_key: &[u8; 32],
    ) -> Result<Self, String> {
        let aid = account_id.to_string();
        let key = *encryption_key;

        let (jmap_url, email, password) = db
            .with_conn(move |conn| read_jmap_credentials(conn, &aid, &key))
            .await?;

        let client = Client::new()
            .credentials((&*email, &*password))
            .connect(&jmap_url)
            .await
            .map_err(|e| format!("JMAP connect failed: {e}"))?;

        Ok(Self {
            inner: Arc::new(client),
        })
    }
}

/// Read JMAP credentials from the database.
///
/// Returns (jmap_url, email, decrypted_password).
fn read_jmap_credentials(
    conn: &rusqlite::Connection,
    account_id: &str,
    key: &[u8; 32],
) -> Result<(String, String, String), String> {
    let mut stmt = conn
        .prepare(
            "SELECT jmap_url, email, imap_password, imap_username \
             FROM accounts WHERE id = ?1",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let row = stmt
        .query_row([account_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| format!("JMAP account {account_id} not found: {e}"))?;

    let jmap_url = row.0.ok_or("No jmap_url configured for account")?;
    let email = row.1;
    let enc_password = row.2.ok_or("No password for JMAP account")?;

    // Use imap_username as login if set, otherwise use email
    let login = row.3.unwrap_or_else(|| email.clone());

    let password = if crypto::is_encrypted(&enc_password) {
        crypto::decrypt_value(key, &enc_password).unwrap_or_else(|_| enc_password.clone())
    } else {
        enc_password
    };

    // We use the login (username or email) for Basic auth
    // but return email separately for display
    let _ = email;

    Ok((jmap_url, login, password))
}

/// Tauri-managed state holding all JMAP clients.
#[derive(Clone)]
pub struct JmapState {
    clients: Arc<RwLock<HashMap<String, JmapClient>>>,
    encryption_key: [u8; 32],
}

impl JmapState {
    pub fn new(encryption_key: [u8; 32]) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            encryption_key,
        }
    }

    /// Get a client for the given account, or return an error if not initialized.
    pub async fn get(&self, account_id: &str) -> Result<JmapClient, String> {
        self.clients
            .read()
            .await
            .get(account_id)
            .cloned()
            .ok_or_else(|| format!("JMAP client not initialized for account {account_id}"))
    }

    /// Insert (or replace) a client for the given account.
    pub async fn insert(&self, account_id: String, client: JmapClient) {
        self.clients.write().await.insert(account_id, client);
    }

    /// Remove the client for the given account.
    pub async fn remove(&self, account_id: &str) {
        self.clients.write().await.remove(account_id);
    }

    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }
}
