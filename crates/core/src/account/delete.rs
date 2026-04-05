use crate::db::Connection;
use crate::db::queries_extra::delete_account_orchestrate_sync;

use super::types::{AccountDeletionData, AccountDeletionPlan};

/// Synchronous phase of full account deletion: gather cleanup data,
/// determine shared references, then delete the account row (which
/// CASCADE-deletes messages, attachments, etc. from the main DB).
///
/// Returns an [`AccountDeletionPlan`] containing everything needed for
/// the subsequent async cleanup of external stores.
pub fn delete_account_orchestrate(
    conn: &Connection,
    account_id: &str,
) -> Result<AccountDeletionPlan, String> {
    delete_account_orchestrate_sync(conn, account_id).map(|plan| AccountDeletionPlan {
        data: AccountDeletionData {
            message_ids: plan.data.message_ids,
            cached_files: plan.data.cached_files,
            inline_hashes: plan.data.inline_hashes,
        },
        shared_cache_hashes: plan.shared_cache_hashes,
        shared_inline_hashes: plan.shared_inline_hashes,
    })
}
