//! Orphan impl of `ProviderSyncOps` for `imap::ops::ImapOps`.

use async_trait::async_trait;
use common::error::ProviderError;
use common::types::SyncResult;
use ::imap::ops::ImapOps;

use crate::{ProviderSyncOps, SyncProviderCtx};

#[async_trait]
impl ProviderSyncOps for ImapOps {
    async fn sync_initial(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx.read_db, ctx.account_id).await?;

        let result = crate::imap::imap_initial::imap_initial_sync(
            ctx.progress,
            ctx.db,
            ctx.read_db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.cancellation_token,
            &account_id,
            &imap_config,
            days_back,
        )
        .await?;

        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn sync_delta(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx.read_db, ctx.account_id).await?;
        let days_back = days_back.unwrap_or(365);

        let result = crate::imap::imap_delta::imap_delta_sync(
            ctx.progress,
            ctx.db,
            ctx.read_db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.cancellation_token,
            &account_id,
            &imap_config,
            days_back,
        )
        .await?;

        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }
}
