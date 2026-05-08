mod action;
mod action_mark_chat_read;
mod action_send;
mod action_status;
mod boot;
mod cal_action;
mod calendar;
mod gal;
mod health;
mod pending_ops_kick;
mod account;
mod attachment;
mod contacts;
pub(crate) mod extract;
mod internal;
mod oauth;
mod pinned_search;
mod settings;
mod signature;
mod smart_folder;
mod sync;
mod thread_ui_state;
#[cfg(feature = "test-helpers")]
mod test_helpers;

pub(crate) use action_mark_chat_read::JournaledChatRead;
pub(crate) use action_send::JournaledSend;

use crate::boot::BootSharedState;
use serde_json::Value;
use service_api::{ClientNotification, RequestParams, ServiceError};
use std::sync::Arc;
use std::time::Instant;

/// Dispatch a request to its handler.
///
/// `RequestParams::Shutdown` is intentionally not handled here - the dispatch
/// loop intercepts it directly so the drain + sentinel + ack ordering is
/// explicit at the lifecycle layer. Treat reaching this arm as a bug.
pub(crate) async fn dispatch(
    params: RequestParams,
    started_at: Instant,
    boot_state: Arc<BootSharedState>,
) -> Result<Value, ServiceError> {
    match params {
        RequestParams::HealthPing => health::handle(started_at).await,
        RequestParams::Shutdown => Err(ServiceError::Internal(
            "shutdown reached handler dispatch; lifecycle layer should have intercepted".into(),
        )),
        RequestParams::BootReady => boot::handle(&boot_state).await,
        RequestParams::ActionExecutePlan { plan } => action::handle(&boot_state, plan).await,
        RequestParams::CalActionExecutePlan { plan } => {
            cal_action::handle(&boot_state, plan).await
        }
        RequestParams::ActionJobStatus { plan_id } => {
            action_status::handle(&boot_state, plan_id).await
        }
        RequestParams::ActionMarkChatRead { chat_email } => {
            action_mark_chat_read::handle(&boot_state, chat_email).await
        }
        RequestParams::ActionSend { request } => action_send::handle(&boot_state, *request).await,
        RequestParams::SyncStartAccount { params } => {
            sync::handle_start_account(&boot_state, params).await
        }
        RequestParams::SyncCancelAccount { params } => {
            sync::handle_cancel_account(&boot_state, params).await
        }
        RequestParams::CalendarStartAccountSync { params } => {
            calendar::handle_start_account_sync(&boot_state, params).await
        }
        RequestParams::CalendarCancelAccountSync { params } => {
            calendar::handle_cancel_account_sync(&boot_state, params).await
        }
        RequestParams::CalendarSetVisibility { params } => {
            calendar::handle_set_visibility(&boot_state, params).await
        }
        RequestParams::ThreadUiStateSet { params } => {
            thread_ui_state::handle_set(&boot_state, params).await
        }
        RequestParams::SettingsSet { params } => {
            settings::handle_set(&boot_state, params).await
        }
        RequestParams::SignatureCreate { params } => {
            signature::handle_create(&boot_state, params).await
        }
        RequestParams::SignatureUpdate { params } => {
            signature::handle_update(&boot_state, params).await
        }
        RequestParams::SignatureDelete { params } => {
            signature::handle_delete(&boot_state, params).await
        }
        RequestParams::SignatureReorder { params } => {
            signature::handle_reorder(&boot_state, params).await
        }
        RequestParams::ContactsGroupSave { params } => {
            contacts::handle_group_save(&boot_state, params).await
        }
        RequestParams::ContactsGroupDelete { params } => {
            contacts::handle_group_delete(&boot_state, params).await
        }
        RequestParams::AccountUpdate { params } => {
            account::handle_update(&boot_state, params).await
        }
        RequestParams::AccountReorder { params } => {
            account::handle_reorder(&boot_state, params).await
        }
        RequestParams::AccountCreate { params } => {
            account::handle_create(&boot_state, params).await
        }
        RequestParams::PinnedSearchCreateOrUpdate { params } => {
            pinned_search::handle_create_or_update(&boot_state, params).await
        }
        RequestParams::PinnedSearchUpdate { params } => {
            pinned_search::handle_update(&boot_state, params).await
        }
        RequestParams::PinnedSearchDelete { params } => {
            pinned_search::handle_delete(&boot_state, params).await
        }
        RequestParams::PinnedSearchDeleteAll { params } => {
            pinned_search::handle_delete_all(&boot_state, params).await
        }
        RequestParams::SmartFolderCreate { params } => {
            smart_folder::handle_create(&boot_state, params).await
        }
        RequestParams::ContactsContactSave { params } => {
            contacts::handle_contact_save(&boot_state, params).await
        }
        RequestParams::ContactsContactSaveWithWriteback { params } => {
            contacts::handle_contact_save_with_writeback(&boot_state, params).await
        }
        RequestParams::ContactsContactDelete { params } => {
            contacts::handle_contact_delete(&boot_state, params).await
        }
        RequestParams::AccountUpdateTokens { params } => {
            account::handle_update_tokens(&boot_state, params).await
        }
        RequestParams::OauthExchangeCode { params } => {
            oauth::handle_exchange_code(&boot_state, params).await
        }
        RequestParams::AttachmentFetch { params } => {
            attachment::handle_fetch(&boot_state, params).await
        }
        RequestParams::ExtractStatus { params } => {
            extract::handle_status(&boot_state, params).await
        }
        RequestParams::IndexRebuild { params } => {
            extract::handle_rebuild(&boot_state, params).await
        }
        RequestParams::AccountDelete { params } => {
            account::handle_delete(&boot_state, params).await
        }
        RequestParams::ReadBootstrapSnapshots { params } => {
            internal::handle_read_bootstrap_snapshots(&boot_state, params).await
        }
        RequestParams::EncryptForStorage { params } => {
            internal::handle_encrypt_for_storage(&boot_state, params).await
        }
        RequestParams::DecryptForStorage { params } => {
            internal::handle_decrypt_for_storage(&boot_state, params).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestPanic => test_helpers::panic_handle().await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestVersion { version } => test_helpers::version_handle(version).await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestSlow { millis } => test_helpers::slow_handle(millis).await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestPrintln { message } => test_helpers::println_handle(message).await,
        #[cfg(feature = "test-helpers")]
        RequestParams::TestSeedAccount { params } => {
            test_helpers::seed_account_handle(&boot_state, params).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestCounterRead { counter } => {
            test_helpers::counter_read_handle(counter).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestCrashAfterNWrites { params } => {
            test_helpers::crash_after_n_writes_handle(params).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestSeedThread { params } => {
            test_helpers::seed_thread_handle(&boot_state, params).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestThreadRead { params } => {
            test_helpers::thread_read_handle(&boot_state, params).await
        }
        #[cfg(feature = "test-helpers")]
        RequestParams::TestDelayNextWrite { params } => {
            test_helpers::delay_next_write_handle(params).await
        }
    }
}

/// Dispatch a UI -> Service notification (Phase 2 plan scope item 11).
///
/// No response is emitted - notifications are fire-and-forget by
/// construction. The handler returns `Result<(), String>` so it can
/// surface a diagnostic into the dispatch log even though the UI never
/// observes it directly.
pub(crate) async fn dispatch_notification(
    notification: ClientNotification,
    boot_state: Arc<BootSharedState>,
) -> Result<(), String> {
    match notification {
        ClientNotification::PendingOpsKick => pending_ops_kick::handle(&boot_state).await,
        ClientNotification::CalendarKick => calendar::handle_calendar_kick(&boot_state).await,
        ClientNotification::GalKick => gal::handle_gal_kick(&boot_state).await,
        ClientNotification::PinnedSearchKick => pinned_search::handle_kick(&boot_state).await,
        ClientNotification::AttachmentEvictionKick => {
            attachment::handle_eviction_kick(&boot_state).await
        }
        ClientNotification::ExtractBackfillKick => {
            extract::handle_backfill_kick(&boot_state).await
        }
    }
}
