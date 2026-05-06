//! Orphan impl of `ProviderSyncOps` for `jmap::ops::JmapOps`.

use async_trait::async_trait;
use common::error::ProviderError;
use common::types::SyncResult;
use jmap::ops::JmapOps;

use crate::{ProviderSyncOps, SyncProviderCtx};

#[async_trait]
impl ProviderSyncOps for JmapOps {
    async fn sync_initial(
        &self,
        ctx: &SyncProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        self.client.ensure_valid_token().await?;
        jmap::sync::jmap_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await?;

        // Sync shared JMAP accounts (discovered from Session in initial sync).
        let shared_results = jmap::shared_mailbox_sync::sync_all_shared_accounts(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await;
        for (id, result) in &shared_results {
            if let Err(e) = result {
                log::warn!("[JMAP] Shared account {id} sync failed during initial: {e}");
            }
        }

        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &SyncProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        self.client.ensure_valid_token().await?;
        let result = jmap::sync::jmap_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await?;

        // Sync shared JMAP accounts after primary delta sync.
        let shared_results = jmap::shared_mailbox_sync::sync_all_shared_accounts(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
            ctx.cancellation_token,
        )
        .await;
        for (id, sr) in &shared_results {
            if let Err(e) = sr {
                log::warn!("[JMAP] Shared account {id} sync failed during delta: {e}");
            }
        }

        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_email_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }
}
