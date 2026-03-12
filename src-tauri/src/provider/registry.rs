use async_trait::async_trait;

use crate::provider::ops::ProviderOps;
use crate::provider::router::get_ops;
use crate::state::ProviderStates;

#[async_trait]
pub trait ProviderRegistry: Send + Sync {
    async fn get_ops(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<Box<dyn ProviderOps>, String>;

    fn encryption_key(&self) -> [u8; 32];
}

#[async_trait]
impl ProviderRegistry for ProviderStates {
    async fn get_ops(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<Box<dyn ProviderOps>, String> {
        get_ops(
            provider,
            account_id,
            self.gmail.as_ref(),
            self.jmap.as_ref(),
            self.graph.as_ref(),
            self.encryption_key(),
        )
        .await
    }

    fn encryption_key(&self) -> [u8; 32] {
        self.encryption_key()
    }
}
