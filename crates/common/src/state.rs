use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use tokio::sync::RwLock;

/// Generic client registry shared by all OAuth-based email providers.
///
/// Each provider (Gmail, Graph, JMAP) type-aliases this with its own client
/// type, e.g. `pub type GmailState = ProviderState<GmailClient>`.
#[derive(Clone)]
pub struct ProviderState<C> {
    clients: Arc<RwLock<HashMap<String, C>>>,
    encryption_key: [u8; 32],
    provider_name: &'static str,
}

impl<C: Clone> ProviderState<C> {
    pub fn new(encryption_key: [u8; 32], provider_name: &'static str) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            encryption_key,
            provider_name,
        }
    }

    /// Get a client for the given account, or return an error if not initialized.
    pub async fn get(&self, account_id: &str) -> Result<C, String> {
        self.clients
            .read()
            .await
            .get(account_id)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "{} client not initialized for account {account_id}",
                    self.provider_name,
                )
            })
    }

    /// Insert (or replace) a client for the given account.
    pub async fn insert(&self, account_id: String, client: C) {
        self.clients.write().await.insert(account_id, client);
    }

    /// Return a cached client, or build and cache one on a miss.
    ///
    /// The second cache check prevents concurrent miss builders from
    /// replacing an already-inserted client. It can still do redundant
    /// construction work under a race, but it preserves the first
    /// cached instance and keeps lock guards out of awaited code.
    pub async fn get_or_try_insert_with<F, Fut>(
        &self,
        account_id: &str,
        build: F,
    ) -> Result<C, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<C, String>>,
    {
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(account_id).cloned() {
                return Ok(client);
            }
        }

        let built = build().await?;

        let mut clients = self.clients.write().await;
        if let Some(client) = clients.get(account_id).cloned() {
            return Ok(client);
        }
        clients.insert(account_id.to_string(), built.clone());
        Ok(built)
    }

    /// Remove the client for the given account.
    pub async fn remove(&self, account_id: &str) {
        self.clients.write().await.remove(account_id);
    }

    pub fn encryption_key(&self) -> &[u8; 32] {
        &self.encryption_key
    }
}
