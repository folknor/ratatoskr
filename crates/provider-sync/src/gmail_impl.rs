//! Orphan impl of `ProviderSyncOps` for `gmail::ops::GmailOps`.
//!
//! Phase 6d-B moved the sync method bodies out of `crates/gmail/src/ops.rs`
//! so the gmail crate no longer needs to depend on `service-state`. The
//! impl logic is unchanged; only the home of the impl block changed.

use ::gmail::ops::GmailOps;
use async_trait::async_trait;
use common::error::ProviderError;
use common::types::SyncResult;

use crate::{ProviderSyncOps, SyncProviderCtx};

#[async_trait]
impl ProviderSyncOps for GmailOps {
    async fn sync_initial(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        crate::gmail::sync::gmail_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.read_db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &SyncProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        let result = crate::gmail::sync::gmail_delta_sync(
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
        .await?;
        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }
}
