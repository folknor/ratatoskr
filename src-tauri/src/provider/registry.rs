use async_trait::async_trait;

use crate::gmail::ops::GmailOps;
use crate::graph::ops::GraphOps;
use crate::imap::ops::ImapOps;
use crate::jmap::ops::JmapOps;
use crate::provider::ops::ProviderOps;
use crate::state::ProviderStates;

#[async_trait]
pub trait ProviderRegistry: Send + Sync {
    async fn get_ops(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<Box<dyn ProviderOps>, String>;
}

#[async_trait]
impl ProviderRegistry for ProviderStates {
    async fn get_ops(
        &self,
        provider: &str,
        account_id: &str,
    ) -> Result<Box<dyn ProviderOps>, String> {
        match provider {
            "gmail_api" => {
                let client = self.gmail.get(account_id).await?;
                Ok(Box::new(GmailOps::new(client)))
            }
            "jmap" => {
                let client = self.jmap.get(account_id).await?;
                Ok(Box::new(JmapOps::new(client)))
            }
            "graph" => {
                let client = self.graph.get(account_id).await?;
                Ok(Box::new(GraphOps::new(client)))
            }
            "imap" => Ok(Box::new(ImapOps::new(self.encryption_key()))),
            other => Err(format!("Unknown provider: {other}")),
        }
    }
}
