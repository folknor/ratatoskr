//! Orphan impl of `ProviderSyncOps` for `graph::ops::GraphOps`.

use async_trait::async_trait;
use common::error::ProviderError;
use common::types::SyncResult;
use ::graph::ops::GraphOps;

use crate::{ProviderSyncOps, SyncProviderCtx};

#[async_trait]
impl ProviderSyncOps for GraphOps {
    async fn sync_initial(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        crate::graph::sync::graph_initial_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.read_db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
            days_back,
        )
        .await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &SyncProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        crate::graph::sync::graph_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.read_db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await
        .map_err(ProviderError::from)
    }
}
