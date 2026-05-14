use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

use crate::action::{ActionWirePlan, PlanId, SendWireRequest};
use crate::cal_action::CalendarActionPlan;
use crate::calendar::{
    CalendarCancelAccountSyncParams, CalendarSetVisibilityParams, CalendarStartAccountSyncParams,
};
use crate::account::{
    AccountCreateParams, AccountDeleteParams, AccountReorderParams, AccountUpdateParams,
    AccountUpdateTokensParams,
};
use crate::contacts::{
    ContactDeleteParams, ContactGroupDeleteParams, ContactGroupSaveParams, ContactSaveParams,
};
use crate::internal::{
    DecryptForStorageParams, EncryptForStorageParams, ReadBootstrapSnapshotsParams,
};
use crate::attachment::{
    AttachmentCacheSizeParams, AttachmentClearCacheParams, AttachmentFetchParams,
};
use crate::extract::{ExtractStatusParams, IndexRebuildParams};
use crate::oauth::OauthExchangeCodeParams;
use crate::pinned_search::{
    PinnedSearchCreateOrUpdateParams, PinnedSearchDeleteAllParams, PinnedSearchDeleteParams,
    PinnedSearchUpdateParams,
};
use crate::settings::SettingsSetParams;
use crate::signature::{
    SignatureCreateParams, SignatureDeleteParams, SignatureReorderParams, SignatureUpdateParams,
};
use crate::smart_folder::SmartFolderCreateParams;
use crate::thread_ui_state::ThreadUiStateSetParams;
use crate::sync::{SyncCancelAccountParams, SyncStartAccountParams};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestTimeoutKind {
    Finite(Duration),
    Infinite,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedAccountParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedAccountAck {
    pub account_id: String,
    pub email: String,
    pub label_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCounterReadAck {
    pub counter: String,
    pub value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCrashAfterNWritesParams {
    pub kind: String,
    pub n: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCrashAfterNWritesAck;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedThreadParams {
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub is_read: bool,
    #[serde(default)]
    pub is_starred: bool,
    #[serde(default)]
    pub is_pinned: bool,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default)]
    pub is_chat_thread: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedThreadAck {
    pub account_id: String,
    pub thread_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedCachedAttachmentParams {
    pub account_id: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedCachedAttachmentAck {
    pub account_id: String,
    pub message_id: String,
    pub attachment_id: String,
    pub content_hash: String,
    pub relative_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedRemoteAttachmentParams {
    pub account_id: String,
    pub message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub content_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSeedRemoteAttachmentAck {
    pub account_id: String,
    pub message_id: String,
    pub attachment_id: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRemoveCachedAttachmentBytesParams {
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRemoveCachedAttachmentBytesAck {
    pub relative_path: String,
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestThreadReadParams {
    pub account_id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestThreadReadAck {
    pub exists: bool,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_pinned: bool,
    pub is_muted: bool,
    pub is_chat_thread: bool,
    pub label_ids: Vec<String>,
    pub unread_messages: u64,
}

/// Phase 8c harness probe params: a single hex-encoded BLAKE3 hash
/// to look up in `attachment_blobs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestQueryBlobTombstoneStateParams {
    pub content_hash: String,
}

/// Phase 8c harness probe ack. `tombstoned_at` is `None` when the
/// row is live or absent. `present` distinguishes "absent from
/// attachment_blobs" from "live in attachment_blobs", since both
/// produce `tombstoned_at = None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestQueryBlobTombstoneStateAck {
    pub present:        bool,
    pub tombstoned_at:  Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPendingOpsReadParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPendingOpRow {
    pub id: String,
    pub account_id: String,
    pub operation_type: String,
    pub resource_id: String,
    pub params: String,
    pub status: String,
    pub retry_count: i64,
    pub max_retries: i64,
    pub next_retry_at: Option<i64>,
    pub created_at: i64,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPendingOpsReadAck {
    pub total: u64,
    pub pending: u64,
    pub failed: u64,
    pub operations: Vec<TestPendingOpRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestStartSyncParams {
    pub account_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestQueryDbStateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact_group_limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbAccountRow {
    pub id: String,
    pub email: String,
    pub provider: String,
    pub auth_method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<i64>,
    pub initial_sync_completed: bool,
    pub access_token_present: bool,
    pub refresh_token_present: bool,
    pub access_token_encrypted: bool,
    pub refresh_token_encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbLabelRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub label_type: String,
    pub label_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_label_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_folder_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_special_use: Option<String>,
    pub sort_order: i64,
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_subscribed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_fg: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbSignatureRow {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
    pub sort_order: i64,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_html_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbMessageRow {
    pub id: String,
    pub account_id: String,
    pub thread_id: String,
    pub subject: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbLocalDraftRow {
    pub id: String,
    pub account_id: String,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_email: Option<String>,
    pub signature_id: Option<String>,
    pub remote_draft_id: Option<String>,
    pub attachments: Option<String>,
    pub signature_separator_index: Option<i64>,
    pub sync_status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbAttachmentRow {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size: Option<i64>,
    pub content_hash: Option<String>,
    pub text_indexed_at: Option<i64>,
    pub extraction_status: Option<String>,
    pub extracted_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbCalendarRow {
    pub id: String,
    pub account_id: String,
    pub provider: String,
    pub remote_id: String,
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub is_primary: bool,
    pub is_visible: bool,
    pub is_default: bool,
    pub provider_id: Option<String>,
    pub can_edit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbCalendarEventRow {
    pub id: String,
    pub account_id: String,
    pub calendar_id: Option<String>,
    /// Legacy DB column name. For non-Google providers this still stores
    /// the provider's remote event id and backs the cross-provider unique key.
    pub google_event_id: String,
    pub remote_event_id: Option<String>,
    pub summary: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub status: Option<String>,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub attendees_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_rule: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbContactRow {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub display_name_overridden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDbContactGroupRow {
    pub id: String,
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestQueryDbStateAck {
    pub account_count: u64,
    pub label_count: u64,
    pub thread_count: u64,
    pub thread_label_count: u64,
    pub message_count: u64,
    pub unread_message_count: u64,
    pub attachment_count: u64,
    pub local_draft_count: u64,
    pub calendar_count: u64,
    pub calendar_event_count: u64,
    #[serde(default)]
    pub contact_count: u64,
    #[serde(default)]
    pub contact_group_count: u64,
    pub accounts: Vec<TestDbAccountRow>,
    #[serde(default)]
    pub labels: Vec<TestDbLabelRow>,
    #[serde(default)]
    pub signatures: Vec<TestDbSignatureRow>,
    pub messages: Vec<TestDbMessageRow>,
    pub local_drafts: Vec<TestDbLocalDraftRow>,
    pub attachments: Vec<TestDbAttachmentRow>,
    pub calendars: Vec<TestDbCalendarRow>,
    pub calendar_events: Vec<TestDbCalendarEventRow>,
    #[serde(default)]
    pub contacts: Vec<TestDbContactRow>,
    #[serde(default)]
    pub contact_groups: Vec<TestDbContactGroupRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestSearchIndexParams {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestSearchIndexResult {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub rank: f32,
    /// Serialized `search::MatchKind` values. Kept as JSON so the
    /// service-api crate does not depend on search internals.
    pub match_kind: Value,
    pub also_matched: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestSearchIndexAck {
    pub total: u64,
    pub results: Vec<TestSearchIndexResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDelayNextWriteParams {
    pub kind: String,
    pub millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestDelayNextWriteAck;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestParams {
    HealthPing,
    Shutdown,
    /// Sent by the UI after the version-check ping; the Service answers it
    /// only after migrations + key load + pending-ops recovery + queued-
    /// drafts sweep + thread-participants backfill have all completed. The
    /// long timeout (10 minutes) covers a 50 GB-class schema migration.
    BootReady,
    /// Submit a resolved-and-planned action for execution. The Service
    /// handler validates the plan, journals it into `action_jobs` +
    /// `action_job_ops` (per Phase 2 plan scope item 18a), signals the
    /// worker pool, and returns `ActionPlanAck { plan_id, journaled }`.
    /// Per-operation `OperationOutcome` notifications stream from the
    /// worker; `ActionCompleted` closes the stream.
    ///
    /// The 5 s timeout is the **handler** budget (validate + insert
    /// rows + signal `tokio::sync::Notify`). The worker has no IPC
    /// timeout - it runs to completion or until respawn.
    ActionExecutePlan { plan: ActionWirePlan },
    /// Look up the journaled status of a previously-submitted plan.
    /// Used by the UI's `AckUnknown` reconciliation path (Phase 2 plan
    /// scope item 11 / 18d): after a `boot.ready` post-respawn, the UI
    /// calls this for every plan whose ack was lost on the wire to
    /// resolve to either `Acked` (Journaled) or `RollBack` (NotFound).
    ///
    /// Read-only SELECT against the journal; the 5 s timeout is
    /// conservative. Doesn't bypass admission - it's just a fast query.
    ActionJobStatus { plan_id: PlanId },
    /// Phase 6c: submit a resolved calendar-event mutation plan for
    /// Service-side execution. The handler validates the plan,
    /// journals it as `kind = 'calendar_plan'` in `action_jobs` +
    /// `action_job_ops` (Phase 6c-1 widened the kind CHECK
    /// constraint), signals the action worker, and returns
    /// `CalendarActionPlanAck { plan_id, journaled }`. Per-op
    /// `CalendarOperationOutcome` notifications stream from the
    /// worker; a final `CalendarActionCompleted` closes the stream.
    ///
    /// 5 s timeout: handler is validate + journal + signal. The
    /// dispatcher runs on the worker, with no IPC timeout - it runs
    /// to completion or until respawn. Same shape as
    /// `ActionExecutePlan` (mail's sibling).
    CalActionExecutePlan { plan: CalendarActionPlan },
    /// Phase 2 plan scope item 18c: the chat read-on-view side effect
    /// relocates as a quiet journal job. Handler resolves affected
    /// threads, runs the local DB write, journals the affected list
    /// for deterministic replay, returns `MarkChatReadAck`. Worker
    /// dispatches provider mark-read against each thread.
    ActionMarkChatRead { chat_email: String },
    /// Phase 2 plan scope item 5: compose-send relocates as a quiet
    /// journal job. Handler validates the request, transfers each
    /// attachment from `<app_data>/staging/<send_id>/` into a
    /// Service-owned vault under `<app_data>/send_vault/<send_id>/`
    /// (atomic rename + SHA-256 verify), journals the send as
    /// `kind = 'send'`, and returns `SendAck`. Worker reads the
    /// journaled vault paths, builds the MIME message, and submits
    /// via SMTP.
    ///
    /// 30 s handler timeout covers SHA-256 verification of typical
    /// attachment payloads (200 MB total verifies in ~400 ms;
    /// gigabyte-class verifies in a few seconds). SMTP upload itself
    /// runs on the worker, not the handler.
    ///
    /// Boxed to keep the `RequestParams` discriminant compact - the
    /// inline-bytes-free `SendWireRequest` is still large (HTML + text
    /// bodies + recipients + attachment metadata for many files) and
    /// would otherwise dominate the enum size.
    ActionSend { request: Box<SendWireRequest> },
    /// Phase 3 plan scope item 1: kick a sync run for the given account.
    /// The handler returns within microseconds (acquires the per-account
    /// map lock, spawns a runner if one is not already in flight, acks).
    /// Sync work runs in the spawned task; the eventual `sync.completed`
    /// notification carries the run's outcome.
    ///
    /// 5 s timeout: the handler is bounded enqueue + spawn work, never
    /// blocking on the network.
    SyncStartAccount { params: SyncStartAccountParams },
    /// Phase 3 plan scope item 1: cancel an in-flight sync run for the
    /// given account. Flips the runner's `CancellationToken`; the runner
    /// observes at the next checkpoint and emits `sync.completed` with
    /// `Cancelled`. The ack carries the active `run_id` so the caller
    /// can subscribe and await the cancellation outcome.
    ///
    /// 5 s timeout: the handler returns immediately after flipping the
    /// token; cancellation propagation is asynchronous.
    SyncCancelAccount { params: SyncCancelAccountParams },
    /// Phase 5: explicit-request calendar sync (manual "Sync now",
    /// post-account-add, RSVP-then-resync). The handler returns within
    /// microseconds: it acquires the per-account map, spawns or returns
    /// an existing runner's id, and acks. The kick-driven path
    /// (cadence + staleness gate) uses `ClientNotification::CalendarKick`
    /// instead and does not surface this request type.
    ///
    /// 5 s timeout: bounded handler work, never blocking on the network.
    CalendarStartAccountSync {
        params: CalendarStartAccountSyncParams,
    },
    /// Phase 5: explicit-request calendar cancel. Account-deletion
    /// cancel is piggybacked server-side inside `handle_cancel_account`
    /// (mirroring push); this request type is reserved for the
    /// explicit-request path.
    ///
    /// 5 s timeout: handler returns immediately after flipping the
    /// runner's cancellation token; cancellation propagation is async.
    CalendarCancelAccountSync {
        params: CalendarCancelAccountSyncParams,
    },
    /// Set the `is_visible` flag on a single `calendars` row. The flat-boolean
    /// half of the calendar UI write surface; event mutations are
    /// Phase 6c.
    ///
    /// 5 s timeout: handler is one bounded `with_conn` write.
    CalendarSetVisibility {
        params: CalendarSetVisibilityParams,
    },
    /// Phase 6a: per-thread UI state writes (`thread_ui_state` table,
    /// keyed on `(account_id, thread_id)`). Today's only field is
    /// `attachments_collapsed`; the IPC carries the full row so future
    /// thread-scoped UI flags can extend without a new method.
    ///
    /// 5 s timeout: handler is one bounded `with_conn` upsert.
    ThreadUiStateSet { params: ThreadUiStateSetParams },
    /// Phase 6a: write one or more settings rows in a single atomic
    /// transaction. The wire shape carries a typed `Vec<SettingValue>`
    /// so the Service-side handler can match exhaustively per variant
    /// (mirrors the project's `MailOperation` discipline). Atomicity
    /// matters because today's caller commits seven preferences at
    /// once - a partial commit would leave the user in an inconsistent
    /// state.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    SettingsSet { params: SettingsSetParams },
    /// Phase 6a: insert one row into `signatures`. Inside a single
    /// transaction the handler also clears `is_default` /
    /// `is_reply_default` on every other signature for the same
    /// account when the new row claims either flag.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    SignatureCreate { params: SignatureCreateParams },
    /// Phase 6a: partial-update one row in `signatures`. Each
    /// `Option` field is "no change" if absent, else "set to value."
    /// Setting `is_default` / `is_reply_default` to `true` clears the
    /// same flag on every other signature for the same account in the
    /// same transaction.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    SignatureUpdate { params: SignatureUpdateParams },
    /// Phase 6a: delete one row from `signatures` by id. Idempotent;
    /// delete-of-missing returns Ok.
    ///
    /// 5 s timeout: handler is one bounded statement.
    SignatureDelete { params: SignatureDeleteParams },
    /// Phase 6a: assign `sort_order` to a flat list of signature ids
    /// in one transaction. Per-account ordering hazard documented on
    /// the wire type - stale acks are tolerable today; a generation
    /// token is the documented escape hatch.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    SignatureReorder { params: SignatureReorderParams },
    /// Phase 6a: UPSERT a contact group + replace its member email
    /// list. The plan's original split (group_create / group_update)
    /// collapsed to one method because today's underlying DB function
    /// is a true UPSERT and the UI always pre-generates ids - see the
    /// `contacts.rs` module doc.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    ContactsGroupSave { params: ContactGroupSaveParams },
    /// Phase 6a: delete a contact group by id. Idempotent;
    /// member rows and inbound nested-group references are cleaned up
    /// inside the same DB transaction.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    ContactsGroupDelete { params: ContactGroupDeleteParams },
    /// Phase 6a-part-2: UPSERT one contact row, local-only. Used by
    /// the bulk-import path (N calls). UI / import path always pre-
    /// generates the id, so the underlying `save_contact_sync` is a
    /// true UPSERT. **No provider write-back** - imports run at
    /// volume and per-row HTTPS would dominate. The single-contact
    /// settings path uses `ContactsContactSaveWithWriteback`.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    ContactsContactSave { params: ContactSaveParams },
    /// Phase 6d-A: full single-contact save pipeline including
    /// provider write-back (JMAP / Google People / Graph) for synced
    /// contacts. Local UPSERT runs first; on local-leg failure the
    /// handler returns `ServiceError`. On provider-leg failure the
    /// ack carries `WritebackOutcome::LocalOnly { reason }` - the
    /// local row is kept, the user-visible state is degraded but not
    /// lost. CardDAV remains a stub returning `LocalOnly`. Replaces
    /// the pre-6d `service::actions::contacts::save_contact` UI-side
    /// call routed through `action_ctx`.
    ///
    /// 30 s timeout: provider HTTPS round-trip dominates the wall
    /// time on a slow link; the local leg is sub-millisecond.
    ContactsContactSaveWithWriteback { params: ContactSaveParams },
    /// Phase 6d-A: full single-contact delete pipeline. Provider-
    /// first for synced JMAP / Google / Graph (matches the pre-6d
    /// UI-side behavior). Provider failure short-circuits before the
    /// local delete and surfaces as `ServiceError`; the local row
    /// stays intact. CardDAV stub returns `LocalOnly`; local-only
    /// contacts (`source = "user"`) delete locally and return
    /// `Success`. Replaces the pre-6d
    /// `service::actions::contacts::delete_contact` UI-side call.
    ///
    /// 30 s timeout: same shape as the writeback save; provider
    /// HTTPS dominates.
    ContactsContactDelete { params: ContactDeleteParams },
    /// Phase 6a: partial-update an account row's editable metadata
    /// fields. Each Option is "no change" if absent, else "set to
    /// value." Out of scope: provider tokens / mailbox passwords -
    /// those mutate via account-create or the future
    /// `internal.encrypt_for_storage` path.
    ///
    /// 5 s timeout: handler is one bounded `dynamic_update`.
    AccountUpdate { params: AccountUpdateParams },
    /// Phase 6a: batch-reassign `sort_order` for accounts. Account
    /// ids absent from `orders` keep their existing `sort_order`.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    AccountReorder { params: AccountReorderParams },
    /// Phase 6a: insert a new account row + companion records.
    /// Credentials carried in a typed `Plaintext | Encrypted` envelope
    /// so 6b's two-step OAuth flow can extend the variant set without
    /// redefining the wire shape. Returns the new account id in the
    /// ack so the UI can kick off post-create flows.
    ///
    /// `Box`ed because `AccountCreateParams` carries ~20 fields and
    /// would dominate the enum's stack size; clippy's
    /// `large_enum_variant` flagged it.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    AccountCreate { params: Box<AccountCreateParams> },
    /// Phase 6a-part-2: re-authentication token persist. Re-issued
    /// from the OAuth or password re-auth flow when an access /
    /// refresh token / IMAP / SMTP password rotates. Service-side
    /// handler runs the dynamic-update SQL that
    /// `update_account_tokens_sync` produces; only the columns
    /// whose `Option` is `Some` are touched.
    ///
    /// 5 s timeout: handler is one bounded UPDATE.
    AccountUpdateTokens { params: Box<AccountUpdateTokensParams> },
    /// Phase 6a-part-2: orchestrated account deletion. The handler
    /// runs cancel-and-await for sync/push/calendar runners (so the
    /// runner-quiescence invariant closes Service-side rather than
    /// being trusted to the caller), then `delete_account_orchestrate`,
    /// then external-store cleanup (body store + inline image store +
    /// search index + attachment file cache), then returns
    /// `AccountDeleteAck` with the cleanup report.
    ///
    /// 60 s timeout: external-store cleanup is the bulk of the work
    /// and routinely runs longer than 5 s on a heavily-cached
    /// account. The 5 s default would surface as spurious timeouts
    /// while the Service is still cleaning up correctly.
    AccountDelete { params: AccountDeleteParams },
    /// Phase 6a-part-2: query-keyed UPSERT into `pinned_searches` +
    /// replacement of the `pinned_search_threads` member rows. The UI
    /// fires this on `SearchPersistenceBehavior::CreatePinnedSnapshot`.
    /// Returns the row id in the ack.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    PinnedSearchCreateOrUpdate { params: PinnedSearchCreateOrUpdateParams },
    /// Phase 6a-part-2: id-keyed UPDATE into `pinned_searches` with a
    /// query-conflict cleanup step. The UI fires this on
    /// `SearchPersistenceBehavior::UpdatePinnedSnapshot` and
    /// `RefreshPinnedSnapshot`.
    ///
    /// 5 s timeout: handler is one bounded transaction.
    PinnedSearchUpdate { params: PinnedSearchUpdateParams },
    /// Phase 6a-part-2: delete a pinned-search row + its member-thread
    /// rows. Idempotent.
    ///
    /// 5 s timeout: handler is one bounded statement.
    PinnedSearchDelete { params: PinnedSearchDeleteParams },
    /// Phase 6a-part-2: clear all pinned searches. Used by the
    /// settings-side "Clear all pinned searches" affordance.
    /// Returns the row count in the ack.
    ///
    /// 5 s timeout: handler is one bounded statement.
    PinnedSearchDeleteAll { params: PinnedSearchDeleteAllParams },
    /// Phase 6a-part-2: insert a new `smart_folders` row. Service
    /// mints the UUID; UI passes only `name` + `query`. Used by the
    /// "Save as smart folder" affordance.
    ///
    /// 5 s timeout: handler is one bounded statement.
    SmartFolderCreate { params: SmartFolderCreateParams },
    /// Phase 6a-part-2 (encryption-key handle): cold-boot read of the
    /// UI + settings bootstrap snapshots, decrypted Service-side. The
    /// handler runs `get_ui_bootstrap_snapshot` and
    /// `get_settings_bootstrap_snapshot` with the in-memory key and
    /// returns the already-decrypted structs as JSON. One round-trip
    /// per cold boot replaces the prior 22+44 per-decrypt local reads.
    ///
    /// 10 s timeout: cold-disk read + AES key-stretch under
    /// contention. Generous because this IPC sits on the cold-boot
    /// critical path and we cannot retry behind the user.
    ReadBootstrapSnapshots { params: ReadBootstrapSnapshotsParams },
    /// Phase 6a-part-2 (encryption-key handle): one-shot encrypt for
    /// credential persistence. Returns the existing
    /// `iv:ciphertext_with_tag` string format that `encrypt_value`
    /// produces. Used by the account-add password persist site and
    /// the rare hand-built persistence in tests.
    ///
    /// 5 s timeout: handler is one in-memory AES encrypt.
    EncryptForStorage { params: EncryptForStorageParams },
    /// Phase 6a-part-2 (encryption-key handle): one-shot decrypt for
    /// the re-auth wizard pre-fill. Returns the plaintext as
    /// `RedactedString` so wire-debug logs cannot leak it.
    ///
    /// 5 s timeout: handler is one in-memory AES decrypt.
    DecryptForStorage { params: DecryptForStorageParams },
    /// Phase 6b: OAuth code-exchange + userinfo round-trips run
    /// Service-side. UI binds the listener and captures the code
    /// locally (the listener has to live in the visible app), then
    /// ships the code via this IPC. When `reauth_account_id` is
    /// `Some`, the handler persists the new tokens onto the named
    /// account row and the ack omits token fields. When `None`, the
    /// ack carries the tokens for the UI to feed into
    /// `account.create` after the Identity step.
    ///
    /// Joins `bypasses_admission()` so the OAuth round-trip is not
    /// queued behind heavy traffic. 30 s timeout: provider token
    /// endpoints are slow under load.
    OauthExchangeCode { params: Box<OauthExchangeCodeParams> },
    /// Phase 6b: cache-miss attachment fetch. Service ensures the
    /// bytes are present in `attachment_cache/<content_hash>`
    /// (cache hit -> immediate ack; cache miss -> provider fetch +
    /// `write_cached` + cache-column update + ack), then returns
    /// the cache-relative path. Bytes never cross the IPC. UI
    /// re-opens the file by relative path; the open fd is the pin
    /// against concurrent eviction (Linux `unlink` is fd-safe).
    ///
    /// 60 s timeout: cache-miss path runs the provider's
    /// fetch_attachment which can be slow on large attachments
    /// over slow links.
    AttachmentFetch { params: AttachmentFetchParams },
    /// Attachments roadmap Phase 6: settings-UI cache-size readout.
    /// Sums live + tombstoned `attachment_blobs.length` for the
    /// "Cache using X.Y GB" indicator.
    AttachmentCacheSize { params: AttachmentCacheSizeParams },
    /// Attachments roadmap Phase 8c: settings-UI "Clear cache now"
    /// action. Tombstones every live blob in bulk and chains a GC
    /// pass to physically reclaim the bytes. Returns counts.
    AttachmentClearCache { params: AttachmentClearCacheParams },
    /// Phase 7-4: read the ExtractRuntime's running status counters
    /// (queue depth + indexed/skipped/failed totals) for the
    /// status-bar polling.
    ExtractStatus { params: ExtractStatusParams },
    /// Phase 7-4: trigger a search-index rebuild. The handler spawns
    /// a tracked task and acks immediately with the `rebuild_id`; the
    /// UI subscribes via `index.rebuild_progress` /
    /// `index.rebuild_completed` notifications.
    IndexRebuild { params: IndexRebuildParams },
    /// Always panics in the handler. Used to verify dispatch panic safety.
    TestPanic,
    /// Returns a `HealthPingResponse` with the requested protocol version.
    /// Used to drive `ClientError::VersionMismatch` from the handshake.
    TestVersion { version: u32 },
    /// Sleeps for `millis` before responding. Used to verify the in-flight
    /// semaphore cap and the heartbeat-bypasses-semaphore property.
    TestSlow { millis: u64 },
    /// Calls `println!` (or its global-stdout-handle equivalent on Windows)
    /// before responding. Used to verify the stdio corruption defense.
    TestPrintln { message: String },
    /// Creates a FK-valid account fixture plus baseline labels.
    TestSeedAccount { params: TestSeedAccountParams },
    /// Reads a service-side test counter by name.
    TestCounterRead { counter: String },
    /// Arms a crash rule that exits after the Nth matching write.
    TestCrashAfterNWrites { params: TestCrashAfterNWritesParams },
    /// Creates a FK-valid thread fixture under a seeded account.
    TestSeedThread { params: TestSeedThreadParams },
    /// Inserts a cached attachment fixture under an existing message.
    TestSeedCachedAttachment { params: TestSeedCachedAttachmentParams },
    /// Inserts an uncached attachment row and registers provider bytes for attachment.fetch.
    TestSeedRemoteAttachment { params: TestSeedRemoteAttachmentParams },
    /// Deletes a cached attachment fixture's backing bytes without changing DB metadata.
    TestRemoveCachedAttachmentBytes { params: TestRemoveCachedAttachmentBytesParams },
    /// Reads back thread flags and labels for harness assertions.
    TestThreadRead { params: TestThreadReadParams },
    /// Reads pending retry-queue rows for harness assertions.
    TestPendingOpsRead { params: TestPendingOpsReadParams },
    /// Starts the real sync runtime from sync-harness scripts.
    TestStartSync { params: TestStartSyncParams },
    /// Reads a small DB snapshot for sync-harness assertions.
    TestQueryDbState { params: TestQueryDbStateParams },
    /// Flushes and queries the test search index.
    TestSearchIndex { params: TestSearchIndexParams },
    /// Arms a one-shot delay at a named write/crash hook.
    TestDelayNextWrite { params: TestDelayNextWriteParams },
    /// Phase 8c harness probe: reads `attachment_blobs.tombstoned_at`
    /// for a single content_hash. Used to verify account-delete /
    /// clear-cache flows from sync-harness scripts after the
    /// referencing `attachments` rows have cascade-deleted away.
    TestQueryBlobTombstoneState { params: TestQueryBlobTombstoneStateParams },
}

/// Phase 8-1: idempotency classification for `RequestParams::idempotency()`.
/// See that method's doc-comment for the per-variant semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// Safe to silently re-issue if the response was lost.
    Idempotent,
    /// Must NOT be replayed; duplicate execution produces a duplicate
    /// observable side effect.
    Mutating,
    /// Idempotent given a known target value; replay is safe if and
    /// only if the target hasn't already been applied. The replay
    /// machinery verifies before re-issuing.
    Conditional,
}

impl RequestParams {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::HealthPing => "health.ping",
            Self::Shutdown => "shutdown",
            Self::BootReady => "boot.ready",
            Self::ActionExecutePlan { .. } => "action.execute_plan",
            Self::CalActionExecutePlan { .. } => "cal_action.execute_plan",
            Self::ActionJobStatus { .. } => "action.job_status",
            Self::ActionMarkChatRead { .. } => "action.mark_chat_read",
            Self::ActionSend { .. } => "action.send",
            Self::SyncStartAccount { .. } => "sync.start_account",
            Self::SyncCancelAccount { .. } => "sync.cancel_account",
            Self::CalendarStartAccountSync { .. } => "calendar.start_account_sync",
            Self::CalendarCancelAccountSync { .. } => "calendar.cancel_account_sync",
            Self::CalendarSetVisibility { .. } => "calendar.set_visibility",
            Self::ThreadUiStateSet { .. } => "thread_ui_state.set",
            Self::SettingsSet { .. } => "settings.set",
            Self::SignatureCreate { .. } => "signature.create",
            Self::SignatureUpdate { .. } => "signature.update",
            Self::SignatureDelete { .. } => "signature.delete",
            Self::SignatureReorder { .. } => "signature.reorder",
            Self::PinnedSearchCreateOrUpdate { .. } => "pinned_search.create_or_update",
            Self::PinnedSearchUpdate { .. } => "pinned_search.update",
            Self::PinnedSearchDelete { .. } => "pinned_search.delete",
            Self::PinnedSearchDeleteAll { .. } => "pinned_search.delete_all",
            Self::SmartFolderCreate { .. } => "smart_folder.create",
            Self::ContactsGroupSave { .. } => "contacts.group_save",
            Self::ContactsGroupDelete { .. } => "contacts.group_delete",
            Self::ContactsContactSave { .. } => "contacts.contact_save",
            Self::ContactsContactSaveWithWriteback { .. } => {
                "contacts.contact_save_with_writeback"
            }
            Self::ContactsContactDelete { .. } => "contacts.contact_delete",
            Self::AccountUpdate { .. } => "account.update",
            Self::AccountReorder { .. } => "account.reorder",
            Self::AccountCreate { .. } => "account.create",
            Self::AccountUpdateTokens { .. } => "account.update_tokens",
            Self::AccountDelete { .. } => "account.delete",
            Self::ReadBootstrapSnapshots { .. } => "internal.read_bootstrap_snapshots",
            Self::EncryptForStorage { .. } => "internal.encrypt_for_storage",
            Self::DecryptForStorage { .. } => "internal.decrypt_for_storage",
            Self::OauthExchangeCode { .. } => "oauth.exchange_code",
            Self::AttachmentFetch { .. } => "attachment.fetch",
            Self::AttachmentCacheSize { .. } => "attachment.cache_size",
            Self::AttachmentClearCache { .. } => "attachment.clear_cache",
            Self::ExtractStatus { .. } => "extract.status",
            Self::IndexRebuild { .. } => "index.rebuild",
            Self::TestPanic => "test.panic",
            Self::TestVersion { .. } => "test.version",
            Self::TestSlow { .. } => "test.slow",
            Self::TestPrintln { .. } => "test.println",
            Self::TestSeedAccount { .. } => "test.seed_account",
            Self::TestCounterRead { .. } => "test.counter_read",
            Self::TestCrashAfterNWrites { .. } => "test.crash_after_n_writes",
            Self::TestSeedThread { .. } => "test.seed_thread",
            Self::TestSeedCachedAttachment { .. } => "test.seed_cached_attachment",
            Self::TestSeedRemoteAttachment { .. } => "test.seed_remote_attachment",
            Self::TestRemoveCachedAttachmentBytes { .. } => {
                "test.remove_cached_attachment_bytes"
            }
            Self::TestThreadRead { .. } => "test.thread_read",
            Self::TestPendingOpsRead { .. } => "test.pending_ops_read",
            Self::TestStartSync { .. } => "test.start_sync",
            Self::TestQueryDbState { .. } => "test.query_db_state",
            Self::TestSearchIndex { .. } => "test.search_index",
            Self::TestDelayNextWrite { .. } => "test.delay_next_write",
            Self::TestQueryBlobTombstoneState { .. } => "test.query_blob_tombstone_state",
        }
    }

    pub fn timeout(&self) -> RequestTimeoutKind {
        // `Shutdown` does NOT set `bypasses_admission()`, but the dispatch
        // loop intercepts it in `handle_line` before reaching the
        // admission check, so the per-handler semaphore and the dispatch-
        // loop admission cap are both effectively bypassed for Shutdown
        // by virtue of dispatch-loop interception. The 30 s timeout below
        // is the budget for the in-flight drain to complete before the
        // UI escalates to SIGTERM.
        match self {
            Self::HealthPing => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::Shutdown => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            Self::BootReady => RequestTimeoutKind::Finite(Duration::from_secs(600)),
            // Handler-only budget: validate + journal + signal worker.
            // The worker has no IPC timeout (per Phase 2 plan scope
            // item 3, which split execution off the request future
            // because the dispatch loop sends the response only after
            // the handler returns).
            Self::ActionExecutePlan { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Same handler-only budget as ActionExecutePlan (mail's
            // sibling): validate + journal + signal worker.
            Self::CalActionExecutePlan { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::ActionJobStatus { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budget: mark_chat_read_local + journal + ack.
            // Provider mark-read happens on the worker.
            Self::ActionMarkChatRead { .. } => RequestTimeoutKind::Finite(Duration::from_secs(10)),
            // Handler budget: validate + per-attachment SHA-256 verify
            // + atomic rename to vault + journal + ack. SMTP is on the
            // worker. 30 s comfortably covers the verify step for
            // realistic attachment sizes (gigabyte-class hashes in a
            // few seconds on commodity hardware).
            Self::ActionSend { .. } => RequestTimeoutKind::Finite(Duration::from_secs(30)),
            // Phase 6d-A: provider HTTPS round-trip dominates. Same
            // shape as ActionSend - the local DB leg is sub-ms; the
            // upstream call is the slow part.
            Self::ContactsContactSaveWithWriteback { .. }
            | Self::ContactsContactDelete { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(30))
            }
            // Handler-only budget: enqueue + spawn (or look up an
            // existing runner and return the ack). No network or DB
            // work in the handler path.
            Self::SyncStartAccount { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budget: flip the token + return the active
            // `run_id`. Cancellation propagation is async.
            Self::SyncCancelAccount { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Handler-only budgets for the calendar request pair.
            // Same shape as the sync pair above.
            Self::CalendarStartAccountSync { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::CalendarCancelAccountSync { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::CalendarSetVisibility { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::ThreadUiStateSet { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::SettingsSet { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::SignatureCreate { .. }
            | Self::SignatureUpdate { .. }
            | Self::SignatureDelete { .. }
            | Self::SignatureReorder { .. }
            | Self::ContactsGroupSave { .. }
            | Self::ContactsGroupDelete { .. }
            | Self::ContactsContactSave { .. }
            | Self::AccountUpdate { .. }
            | Self::AccountUpdateTokens { .. }
            | Self::AccountReorder { .. }
            | Self::AccountCreate { .. }
            | Self::PinnedSearchCreateOrUpdate { .. }
            | Self::PinnedSearchUpdate { .. }
            | Self::PinnedSearchDelete { .. }
            | Self::PinnedSearchDeleteAll { .. }
            | Self::SmartFolderCreate { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            // External-store cleanup is the bulk of the work and
            // routinely exceeds the 5 s default on a heavily-cached
            // account. 60 s absorbs that without converting correct
            // cleanup into a spurious timeout.
            Self::AccountDelete { .. } => RequestTimeoutKind::Finite(Duration::from_secs(60)),
            // Cold-boot critical path; absorb cold-disk + key-stretch.
            Self::ReadBootstrapSnapshots { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(10))
            }
            Self::EncryptForStorage { .. } | Self::DecryptForStorage { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            // Provider token endpoints can be slow under load; the
            // round-trip is two HTTPS calls (token + userinfo) plus
            // optional re-auth DB write.
            Self::OauthExchangeCode { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(30))
            }
            // Cache miss runs the provider's fetch_attachment, which
            // can be slow on large attachments over slow links.
            Self::AttachmentFetch { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(60))
            }
            // Single SQL aggregate over `attachment_blobs`; sub-second
            // on any realistic cache.
            Self::AttachmentCacheSize { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            // Phase 8c: bulk tombstone + GC pass. Both are typically
            // sub-second on a small mailbox but can grow with cache
            // size; allow plenty of headroom so an honest IPC ack
            // doesn't get timed out on a 150 GB cache.
            Self::AttachmentClearCache { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(300))
            }
            // Phase 7-4: in-memory counter read; cheap.
            Self::ExtractStatus { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            // Phase 7-4: handler spawns a tracked task and returns
            // immediately with the rebuild_id; the rebuild itself runs
            // asynchronously.
            Self::IndexRebuild { .. } => RequestTimeoutKind::Finite(Duration::from_secs(5)),
            Self::TestPanic | Self::TestVersion { .. } | Self::TestPrintln { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::TestSeedAccount { .. }
            | Self::TestCounterRead { .. }
            | Self::TestCrashAfterNWrites { .. }
            | Self::TestSeedThread { .. }
            | Self::TestSeedCachedAttachment { .. }
            | Self::TestSeedRemoteAttachment { .. }
            | Self::TestRemoveCachedAttachmentBytes { .. }
            | Self::TestThreadRead { .. }
            | Self::TestPendingOpsRead { .. }
            | Self::TestStartSync { .. }
            | Self::TestQueryDbState { .. }
            | Self::TestSearchIndex { .. }
            | Self::TestDelayNextWrite { .. }
            | Self::TestQueryBlobTombstoneState { .. } => {
                RequestTimeoutKind::Finite(Duration::from_secs(5))
            }
            Self::TestSlow { .. } => RequestTimeoutKind::Finite(Duration::from_secs(60)),
        }
    }

    /// Phase 8-1: per-method idempotency classification. Intended for
    /// the future client-side replay-on-respawn machinery (T1 cohort,
    /// harness M4); 8-1 itself only records the contract. Today the
    /// client unconditionally fails pending requests with
    /// `ClientError::ServiceCrashed` on respawn; once replay lands, the
    /// classification picks which path each method takes.
    ///
    /// - `Idempotent`: safe to silently re-issue if the response was
    ///   lost on the wire pre-respawn. Pure reads or "compute from
    ///   inputs" RPCs.
    /// - `Mutating`: must NOT be replayed - duplicate execution
    ///   produces a duplicate observable side effect (sends an email
    ///   twice, deletes an account twice, consumes the same OAuth
    ///   authorization code twice). The client must surface the loss
    ///   as `ServiceCrashed` and let the user retry.
    /// - `Conditional`: idempotent given a known target value (the
    ///   "set state to X" shape). Replay is safe if and only if the
    ///   target hasn't been observed pre-crash. The replay machinery
    ///   verifies post-respawn before re-issuing; if the target was
    ///   already applied, the replay becomes a no-op.
    pub fn idempotency(&self) -> Idempotency {
        match self {
            Self::HealthPing
            | Self::BootReady
            | Self::ActionJobStatus { .. }
            | Self::ReadBootstrapSnapshots { .. }
            | Self::EncryptForStorage { .. }
            | Self::DecryptForStorage { .. }
            | Self::AttachmentFetch { .. }
            | Self::AttachmentCacheSize { .. }
            // Phase 8c: clear-cache is idempotent. A second call
            // returning (blobs_tombstoned=0, bytes_reclaimed=0) is
            // the correct honest answer to "did the wipe finish?"
            // when the first call's IPC ack was lost mid-flight; a
            // non-idempotent classification would block the
            // dispatcher's retry and leave the UI without a signal
            // that the work succeeded.
            | Self::AttachmentClearCache { .. }
            | Self::ExtractStatus { .. } => Idempotency::Idempotent,

            Self::Shutdown
            | Self::ActionExecutePlan { .. }
            | Self::CalActionExecutePlan { .. }
            | Self::ActionSend { .. }
            | Self::OauthExchangeCode { .. }
            | Self::SyncCancelAccount { .. }
            | Self::CalendarCancelAccountSync { .. }
            | Self::SignatureCreate { .. }
            | Self::SmartFolderCreate { .. }
            | Self::ContactsGroupSave { .. }
            | Self::ContactsGroupDelete { .. }
            | Self::ContactsContactSave { .. }
            | Self::ContactsContactSaveWithWriteback { .. }
            | Self::ContactsContactDelete { .. }
            | Self::AccountCreate { .. }
            | Self::AccountDelete { .. }
            | Self::PinnedSearchCreateOrUpdate { .. } => Idempotency::Mutating,

            Self::ActionMarkChatRead { .. }
            | Self::SyncStartAccount { .. }
            | Self::CalendarStartAccountSync { .. }
            | Self::CalendarSetVisibility { .. }
            | Self::ThreadUiStateSet { .. }
            | Self::SettingsSet { .. }
            | Self::SignatureUpdate { .. }
            | Self::SignatureDelete { .. }
            | Self::SignatureReorder { .. }
            | Self::PinnedSearchUpdate { .. }
            | Self::PinnedSearchDelete { .. }
            | Self::PinnedSearchDeleteAll { .. }
            | Self::AccountUpdate { .. }
            | Self::AccountReorder { .. }
            | Self::AccountUpdateTokens { .. }
            | Self::IndexRebuild { .. } => Idempotency::Conditional,

            Self::TestPanic
            | Self::TestVersion { .. }
            | Self::TestSlow { .. }
            | Self::TestPrintln { .. }
            | Self::TestCounterRead { .. }
            | Self::TestThreadRead { .. }
            | Self::TestPendingOpsRead { .. }
            | Self::TestQueryDbState { .. }
            | Self::TestSearchIndex { .. }
            | Self::TestQueryBlobTombstoneState { .. } => Idempotency::Idempotent,

            Self::TestSeedAccount { .. }
            | Self::TestCrashAfterNWrites { .. }
            | Self::TestSeedThread { .. }
            | Self::TestSeedCachedAttachment { .. }
            | Self::TestSeedRemoteAttachment { .. }
            | Self::TestRemoveCachedAttachmentBytes { .. }
            | Self::TestDelayNextWrite { .. } => Idempotency::Mutating,

            Self::TestStartSync { .. } => Idempotency::Conditional,
        }
    }

    /// Requests that bypass BOTH the per-handler semaphore and the dispatch-
    /// loop admission cap.
    ///
    /// `health.ping` keeps the heartbeat alive under load; `boot.ready` is
    /// special-cased because it parks on a `Notify` until the boot sequence
    /// completes (occupying a semaphore permit while parked would let a long
    /// migration starve other handlers) and because flooding the dispatch
    /// loop with slow requests would otherwise be able to push the boot
    /// handshake out past the admission cap.
    ///
    /// `oauth.exchange_code` bypasses because it makes an external HTTPS
    /// call (token exchange + userinfo) inside the handler future. It runs
    /// at human-paced cadence (account create / re-auth flow) so contention
    /// is not a real-world concern, but the 30 s timeout would otherwise
    /// pin a semaphore permit while waiting on a slow / wedged provider.
    /// The bypass keeps the rest of the dispatch loop responsive during
    /// the round-trip.
    ///
    /// Renamed from `bypasses_semaphore` in Phase 1.5 to reflect the dual
    /// role - the dispatch loop's `ADMISSION_CAP` gate also keys off this
    /// flag.
    pub fn bypasses_admission(&self) -> bool {
        matches!(
            self,
            Self::HealthPing | Self::BootReady | Self::OauthExchangeCode { .. },
        )
    }

    /// Serialize this request's params into the `params` field of the
    /// JSON-RPC envelope.
    ///
    /// Unit variants serialize to `Value::Null` (the wire-canonical "no
    /// params"). Tuple-shaped variants serialize their inner struct via
    /// `serde_json::to_value`. Each match arm is the canonical extension
    /// point.
    pub fn params_value(&self) -> Value {
        match self {
            Self::HealthPing => Value::Null,
            Self::Shutdown => Value::Null,
            Self::BootReady => Value::Null,
            Self::ActionExecutePlan { plan } => serde_json::json!({ "plan": plan }),
            Self::CalActionExecutePlan { plan } => serde_json::json!({ "plan": plan }),
            Self::ActionJobStatus { plan_id } => serde_json::json!({ "plan_id": plan_id }),
            Self::ActionMarkChatRead { chat_email } => {
                serde_json::json!({ "chat_email": chat_email })
            }
            Self::ActionSend { request } => serde_json::json!({ "request": request }),
            Self::SyncStartAccount { params } => serde_json::json!({ "params": params }),
            Self::SyncCancelAccount { params } => serde_json::json!({ "params": params }),
            Self::CalendarStartAccountSync { params } => serde_json::json!({ "params": params }),
            Self::CalendarCancelAccountSync { params } => {
                serde_json::json!({ "params": params })
            }
            Self::CalendarSetVisibility { params } => serde_json::json!({ "params": params }),
            Self::ThreadUiStateSet { params } => serde_json::json!({ "params": params }),
            Self::SettingsSet { params } => serde_json::json!({ "params": params }),
            Self::SignatureCreate { params } => serde_json::json!({ "params": params }),
            Self::SignatureUpdate { params } => serde_json::json!({ "params": params }),
            Self::SignatureDelete { params } => serde_json::json!({ "params": params }),
            Self::SignatureReorder { params } => serde_json::json!({ "params": params }),
            Self::PinnedSearchCreateOrUpdate { params } => serde_json::json!({ "params": params }),
            Self::PinnedSearchUpdate { params } => serde_json::json!({ "params": params }),
            Self::PinnedSearchDelete { params } => serde_json::json!({ "params": params }),
            Self::PinnedSearchDeleteAll { params } => serde_json::json!({ "params": params }),
            Self::SmartFolderCreate { params } => serde_json::json!({ "params": params }),
            Self::ContactsGroupSave { params } => serde_json::json!({ "params": params }),
            Self::ContactsGroupDelete { params } => serde_json::json!({ "params": params }),
            Self::ContactsContactSave { params } => serde_json::json!({ "params": params }),
            Self::ContactsContactSaveWithWriteback { params } => {
                serde_json::json!({ "params": params })
            }
            Self::ContactsContactDelete { params } => serde_json::json!({ "params": params }),
            Self::AccountUpdate { params } => serde_json::json!({ "params": params }),
            Self::AccountReorder { params } => serde_json::json!({ "params": params }),
            Self::AccountCreate { params } => serde_json::json!({ "params": params }),
            Self::AccountUpdateTokens { params } => serde_json::json!({ "params": params }),
            Self::AccountDelete { params } => serde_json::json!({ "params": params }),
            Self::ReadBootstrapSnapshots { params } => serde_json::json!({ "params": params }),
            Self::EncryptForStorage { params } => serde_json::json!({ "params": params }),
            Self::DecryptForStorage { params } => serde_json::json!({ "params": params }),
            Self::OauthExchangeCode { params } => serde_json::json!({ "params": params }),
            Self::AttachmentFetch { params } => serde_json::json!({ "params": params }),
            Self::AttachmentCacheSize { params } => serde_json::json!({ "params": params }),
            Self::AttachmentClearCache { params } => serde_json::json!({ "params": params }),
            Self::ExtractStatus { params } => serde_json::json!({ "params": params }),
            Self::IndexRebuild { params } => serde_json::json!({ "params": params }),
            Self::TestPanic => Value::Null,
            Self::TestVersion { version } => serde_json::json!({ "version": version }),
            Self::TestSlow { millis } => serde_json::json!({ "millis": millis }),
            Self::TestPrintln { message } => serde_json::json!({ "message": message }),
            Self::TestSeedAccount { params } => serde_json::json!({ "params": params }),
            Self::TestCounterRead { counter } => serde_json::json!({ "counter": counter }),
            Self::TestCrashAfterNWrites { params } => {
                serde_json::json!({ "params": params })
            }
            Self::TestSeedThread { params } => serde_json::json!({ "params": params }),
            Self::TestSeedCachedAttachment { params } => {
                serde_json::json!({ "params": params })
            }
            Self::TestSeedRemoteAttachment { params } => {
                serde_json::json!({ "params": params })
            }
            Self::TestRemoveCachedAttachmentBytes { params } => {
                serde_json::json!({ "params": params })
            }
            Self::TestThreadRead { params } => serde_json::json!({ "params": params }),
            Self::TestPendingOpsRead { params } => serde_json::json!({ "params": params }),
            Self::TestStartSync { params } => serde_json::json!({ "params": params }),
            Self::TestQueryDbState { params } => serde_json::json!({ "params": params }),
            Self::TestSearchIndex { params } => serde_json::json!({ "params": params }),
            Self::TestDelayNextWrite { params } => {
                serde_json::json!({ "params": params })
            }
            Self::TestQueryBlobTombstoneState { params } => {
                serde_json::json!({ "params": params })
            }
        }
    }

    pub fn from_method_params(method: &str, params: Option<Value>) -> Result<Self, String> {
        match method {
            "health.ping" => {
                expect_no_params(method, params)?;
                Ok(Self::HealthPing)
            }
            "shutdown" => {
                expect_no_params(method, params)?;
                Ok(Self::Shutdown)
            }
            "boot.ready" => {
                expect_no_params(method, params)?;
                Ok(Self::BootReady)
            }
            "action.execute_plan" => {
                #[derive(Deserialize)]
                struct P {
                    plan: ActionWirePlan,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.execute_plan params: {e}"))?;
                Ok(Self::ActionExecutePlan { plan: p.plan })
            }
            "cal_action.execute_plan" => {
                #[derive(Deserialize)]
                struct P {
                    plan: CalendarActionPlan,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("cal_action.execute_plan params: {e}"))?;
                Ok(Self::CalActionExecutePlan { plan: p.plan })
            }
            "action.job_status" => {
                #[derive(Deserialize)]
                struct P {
                    plan_id: PlanId,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.job_status params: {e}"))?;
                Ok(Self::ActionJobStatus { plan_id: p.plan_id })
            }
            "action.mark_chat_read" => {
                #[derive(Deserialize)]
                struct P {
                    chat_email: String,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.mark_chat_read params: {e}"))?;
                Ok(Self::ActionMarkChatRead {
                    chat_email: p.chat_email,
                })
            }
            "action.send" => {
                #[derive(Deserialize)]
                struct P {
                    request: SendWireRequest,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("action.send params: {e}"))?;
                Ok(Self::ActionSend {
                    request: Box::new(p.request),
                })
            }
            "sync.start_account" => {
                #[derive(Deserialize)]
                struct P {
                    params: SyncStartAccountParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("sync.start_account params: {e}"))?;
                Ok(Self::SyncStartAccount { params: p.params })
            }
            "sync.cancel_account" => {
                #[derive(Deserialize)]
                struct P {
                    params: SyncCancelAccountParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("sync.cancel_account params: {e}"))?;
                Ok(Self::SyncCancelAccount { params: p.params })
            }
            "calendar.start_account_sync" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarStartAccountSyncParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.start_account_sync params: {e}"))?;
                Ok(Self::CalendarStartAccountSync { params: p.params })
            }
            "calendar.cancel_account_sync" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarCancelAccountSyncParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.cancel_account_sync params: {e}"))?;
                Ok(Self::CalendarCancelAccountSync { params: p.params })
            }
            "calendar.set_visibility" => {
                #[derive(Deserialize)]
                struct P {
                    params: CalendarSetVisibilityParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("calendar.set_visibility params: {e}"))?;
                Ok(Self::CalendarSetVisibility { params: p.params })
            }
            "thread_ui_state.set" => {
                #[derive(Deserialize)]
                struct P {
                    params: ThreadUiStateSetParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("thread_ui_state.set params: {e}"))?;
                Ok(Self::ThreadUiStateSet { params: p.params })
            }
            "settings.set" => {
                #[derive(Deserialize)]
                struct P {
                    params: SettingsSetParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("settings.set params: {e}"))?;
                Ok(Self::SettingsSet { params: p.params })
            }
            "signature.create" => {
                #[derive(Deserialize)]
                struct P {
                    params: SignatureCreateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("signature.create params: {e}"))?;
                Ok(Self::SignatureCreate { params: p.params })
            }
            "signature.update" => {
                #[derive(Deserialize)]
                struct P {
                    params: SignatureUpdateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("signature.update params: {e}"))?;
                Ok(Self::SignatureUpdate { params: p.params })
            }
            "signature.delete" => {
                #[derive(Deserialize)]
                struct P {
                    params: SignatureDeleteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("signature.delete params: {e}"))?;
                Ok(Self::SignatureDelete { params: p.params })
            }
            "signature.reorder" => {
                #[derive(Deserialize)]
                struct P {
                    params: SignatureReorderParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("signature.reorder params: {e}"))?;
                Ok(Self::SignatureReorder { params: p.params })
            }
            "contacts.group_save" => {
                #[derive(Deserialize)]
                struct P {
                    params: ContactGroupSaveParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("contacts.group_save params: {e}"))?;
                Ok(Self::ContactsGroupSave { params: p.params })
            }
            "contacts.group_delete" => {
                #[derive(Deserialize)]
                struct P {
                    params: ContactGroupDeleteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("contacts.group_delete params: {e}"))?;
                Ok(Self::ContactsGroupDelete { params: p.params })
            }
            "contacts.contact_save" => {
                #[derive(Deserialize)]
                struct P {
                    params: ContactSaveParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("contacts.contact_save params: {e}"))?;
                Ok(Self::ContactsContactSave { params: p.params })
            }
            "contacts.contact_save_with_writeback" => {
                #[derive(Deserialize)]
                struct P {
                    params: ContactSaveParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("contacts.contact_save_with_writeback params: {e}"))?;
                Ok(Self::ContactsContactSaveWithWriteback { params: p.params })
            }
            "contacts.contact_delete" => {
                #[derive(Deserialize)]
                struct P {
                    params: ContactDeleteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("contacts.contact_delete params: {e}"))?;
                Ok(Self::ContactsContactDelete { params: p.params })
            }
            "account.update" => {
                #[derive(Deserialize)]
                struct P {
                    params: AccountUpdateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("account.update params: {e}"))?;
                Ok(Self::AccountUpdate { params: p.params })
            }
            "account.reorder" => {
                #[derive(Deserialize)]
                struct P {
                    params: AccountReorderParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("account.reorder params: {e}"))?;
                Ok(Self::AccountReorder { params: p.params })
            }
            "account.create" => {
                #[derive(Deserialize)]
                struct P {
                    params: AccountCreateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("account.create params: {e}"))?;
                Ok(Self::AccountCreate {
                    params: Box::new(p.params),
                })
            }
            "account.delete" => {
                #[derive(Deserialize)]
                struct P {
                    params: AccountDeleteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("account.delete params: {e}"))?;
                Ok(Self::AccountDelete { params: p.params })
            }
            "account.update_tokens" => {
                #[derive(Deserialize)]
                struct P {
                    params: AccountUpdateTokensParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("account.update_tokens params: {e}"))?;
                Ok(Self::AccountUpdateTokens {
                    params: Box::new(p.params),
                })
            }
            "pinned_search.create_or_update" => {
                #[derive(Deserialize)]
                struct P {
                    params: PinnedSearchCreateOrUpdateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("pinned_search.create_or_update params: {e}"))?;
                Ok(Self::PinnedSearchCreateOrUpdate { params: p.params })
            }
            "pinned_search.update" => {
                #[derive(Deserialize)]
                struct P {
                    params: PinnedSearchUpdateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("pinned_search.update params: {e}"))?;
                Ok(Self::PinnedSearchUpdate { params: p.params })
            }
            "pinned_search.delete" => {
                #[derive(Deserialize)]
                struct P {
                    params: PinnedSearchDeleteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("pinned_search.delete params: {e}"))?;
                Ok(Self::PinnedSearchDelete { params: p.params })
            }
            "pinned_search.delete_all" => {
                #[derive(Deserialize)]
                struct P {
                    params: PinnedSearchDeleteAllParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("pinned_search.delete_all params: {e}"))?;
                Ok(Self::PinnedSearchDeleteAll { params: p.params })
            }
            "smart_folder.create" => {
                #[derive(Deserialize)]
                struct P {
                    params: SmartFolderCreateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("smart_folder.create params: {e}"))?;
                Ok(Self::SmartFolderCreate { params: p.params })
            }
            "internal.read_bootstrap_snapshots" => {
                #[derive(Deserialize)]
                struct P {
                    params: ReadBootstrapSnapshotsParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("internal.read_bootstrap_snapshots params: {e}"))?;
                Ok(Self::ReadBootstrapSnapshots { params: p.params })
            }
            "internal.encrypt_for_storage" => {
                #[derive(Deserialize)]
                struct P {
                    params: EncryptForStorageParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("internal.encrypt_for_storage params: {e}"))?;
                Ok(Self::EncryptForStorage { params: p.params })
            }
            "internal.decrypt_for_storage" => {
                #[derive(Deserialize)]
                struct P {
                    params: DecryptForStorageParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("internal.decrypt_for_storage params: {e}"))?;
                Ok(Self::DecryptForStorage { params: p.params })
            }
            "oauth.exchange_code" => {
                #[derive(Deserialize)]
                struct P {
                    params: OauthExchangeCodeParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("oauth.exchange_code params: {e}"))?;
                Ok(Self::OauthExchangeCode {
                    params: Box::new(p.params),
                })
            }
            "attachment.fetch" => {
                #[derive(Deserialize)]
                struct P {
                    params: AttachmentFetchParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("attachment.fetch params: {e}"))?;
                Ok(Self::AttachmentFetch { params: p.params })
            }
            "attachment.cache_size" => {
                #[derive(Deserialize, Default)]
                struct P {
                    #[serde(default)]
                    params: AttachmentCacheSizeParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .unwrap_or_default();
                Ok(Self::AttachmentCacheSize { params: p.params })
            }
            "attachment.clear_cache" => {
                #[derive(Deserialize, Default)]
                struct P {
                    #[serde(default)]
                    params: AttachmentClearCacheParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .unwrap_or_default();
                Ok(Self::AttachmentClearCache { params: p.params })
            }
            "extract.status" => {
                #[derive(Deserialize)]
                struct P {
                    params: ExtractStatusParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("extract.status params: {e}"))?;
                Ok(Self::ExtractStatus { params: p.params })
            }
            "index.rebuild" => {
                #[derive(Deserialize)]
                struct P {
                    params: IndexRebuildParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("index.rebuild params: {e}"))?;
                Ok(Self::IndexRebuild { params: p.params })
            }
            "test.panic" => {
                expect_no_params(method, params)?;
                Ok(Self::TestPanic)
            }
            "test.version" => {
                #[derive(Deserialize)]
                struct P {
                    version: u32,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.version params: {e}"))?;
                Ok(Self::TestVersion { version: p.version })
            }
            "test.slow" => {
                #[derive(Deserialize)]
                struct P {
                    millis: u64,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.slow params: {e}"))?;
                Ok(Self::TestSlow { millis: p.millis })
            }
            "test.println" => {
                #[derive(Deserialize)]
                struct P {
                    message: String,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.println params: {e}"))?;
                Ok(Self::TestPrintln { message: p.message })
            }
            "test.seed_account" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestSeedAccountParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.seed_account params: {e}"))?;
                Ok(Self::TestSeedAccount { params: p.params })
            }
            "test.counter_read" => {
                #[derive(Deserialize)]
                struct P {
                    counter: String,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.counter_read params: {e}"))?;
                Ok(Self::TestCounterRead { counter: p.counter })
            }
            "test.crash_after_n_writes" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestCrashAfterNWritesParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.crash_after_n_writes params: {e}"))?;
                Ok(Self::TestCrashAfterNWrites { params: p.params })
            }
            "test.seed_thread" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestSeedThreadParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.seed_thread params: {e}"))?;
                Ok(Self::TestSeedThread { params: p.params })
            }
            "test.seed_cached_attachment" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestSeedCachedAttachmentParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.seed_cached_attachment params: {e}"))?;
                Ok(Self::TestSeedCachedAttachment { params: p.params })
            }
            "test.seed_remote_attachment" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestSeedRemoteAttachmentParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.seed_remote_attachment params: {e}"))?;
                Ok(Self::TestSeedRemoteAttachment { params: p.params })
            }
            "test.remove_cached_attachment_bytes" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestRemoveCachedAttachmentBytesParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.remove_cached_attachment_bytes params: {e}"))?;
                Ok(Self::TestRemoveCachedAttachmentBytes { params: p.params })
            }
            "test.thread_read" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestThreadReadParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.thread_read params: {e}"))?;
                Ok(Self::TestThreadRead { params: p.params })
            }
            "test.pending_ops_read" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestPendingOpsReadParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.pending_ops_read params: {e}"))?;
                Ok(Self::TestPendingOpsRead { params: p.params })
            }
            "test.start_sync" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestStartSyncParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.start_sync params: {e}"))?;
                Ok(Self::TestStartSync { params: p.params })
            }
            "test.query_db_state" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestQueryDbStateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.query_db_state params: {e}"))?;
                Ok(Self::TestQueryDbState { params: p.params })
            }
            "test.search_index" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestSearchIndexParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.search_index params: {e}"))?;
                Ok(Self::TestSearchIndex { params: p.params })
            }
            "test.delay_next_write" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestDelayNextWriteParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.delay_next_write params: {e}"))?;
                Ok(Self::TestDelayNextWrite { params: p.params })
            }
            "test.query_blob_tombstone_state" => {
                #[derive(Deserialize)]
                struct P {
                    params: TestQueryBlobTombstoneStateParams,
                }
                let p: P = serde_json::from_value(params.unwrap_or(Value::Null))
                    .map_err(|e| format!("test.query_blob_tombstone_state params: {e}"))?;
                Ok(Self::TestQueryBlobTombstoneState { params: p.params })
            }
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

/// For unit variants that take no params. Future struct-shaped variants
/// should `serde_json::from_value::<TheirParams>(params.unwrap_or(Null))`
/// instead.
fn expect_no_params(method: &str, params: Option<Value>) -> Result<(), String> {
    match params {
        None => Ok(()),
        Some(Value::Object(map)) if map.is_empty() => Ok(()),
        Some(Value::Null) => Ok(()),
        Some(_) => Err(format!("{method} expects no params")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_ready_timeout_is_ten_minutes() {
        assert_eq!(
            RequestParams::BootReady.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(600)),
        );
    }

    #[test]
    fn boot_ready_method_name_is_dotted() {
        assert_eq!(RequestParams::BootReady.method_name(), "boot.ready");
    }

    #[test]
    fn boot_ready_bypasses_admission() {
        assert!(RequestParams::BootReady.bypasses_admission());
    }

    #[test]
    fn health_ping_bypasses_admission() {
        assert!(RequestParams::HealthPing.bypasses_admission());
    }

    #[test]
    fn shutdown_does_not_bypass_admission() {
        assert!(!RequestParams::Shutdown.bypasses_admission());
    }

    #[test]
    fn action_execute_plan_timeout_is_five_seconds() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionExecutePlan { plan }.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn action_execute_plan_method_name_is_dotted() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionExecutePlan { plan }.method_name(),
            "action.execute_plan",
        );
    }

    #[test]
    fn action_execute_plan_does_not_bypass_admission() {
        let plan = ActionWirePlan {
            plan_id: crate::action::PlanId::new_v7(),
            operations: Vec::new(),
        };
        assert!(
            !RequestParams::ActionExecutePlan { plan }.bypasses_admission(),
            "action.execute_plan is bounded handler work; admission cap applies",
        );
    }

    #[test]
    fn action_execute_plan_round_trips_from_method_params() {
        use crate::action::{
            ActionWireOperation, OperationId, PlanId, WireFolderId, WireMailOperation,
        };

        let plan = ActionWirePlan {
            plan_id: PlanId::new_v7(),
            operations: vec![
                ActionWireOperation {
                    operation_id: OperationId(0),
                    account_id: "acc-1".into(),
                    thread_id: "thr-9".into(),
                    operation: WireMailOperation::MoveToFolder {
                        dest: WireFolderId("inbox".into()),
                        source: Some(WireFolderId("archive".into())),
                    },
                },
            ],
        };
        let original = RequestParams::ActionExecutePlan { plan };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn cal_action_execute_plan_round_trips_from_method_params() {
        use crate::action::{OperationId, PlanId};
        use crate::cal_action::{
            CalendarActionPlan, CalendarActionWireOperation, WireCalendarEventInput,
            WireCalendarOperation,
        };

        let plan = CalendarActionPlan {
            plan_id: PlanId::new_v7(),
            operations: vec![CalendarActionWireOperation {
                operation_id: OperationId(0),
                account_id: "acc-1".into(),
                operation: WireCalendarOperation::CreateEvent {
                    calendar_remote_id: "primary".into(),
                    input: WireCalendarEventInput {
                        title: "Standup".into(),
                        description: String::new(),
                        location: String::new(),
                        start_time: 1_700_000_000,
                        end_time: 1_700_003_600,
                        is_all_day: false,
                        timezone: Some("UTC".into()),
                        recurrence_rule: None,
                        availability: None,
                        visibility: None,
                    },
                },
            }],
        };
        let original = RequestParams::CalActionExecutePlan { plan };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "cal_action.execute_plan");
        assert_eq!(
            original.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
        assert!(
            !original.bypasses_admission(),
            "cal_action.execute_plan must not bypass admission",
        );
    }

    #[test]
    fn action_send_method_name_is_dotted() {
        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: crate::action::SendWireMessage {
                draft_id: "d".into(),
                from: "a@b".into(),
                to: vec!["c@d".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: None,
                body_html: String::new(),
                body_text: String::new(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionSend {
                request: Box::new(req),
            }
            .method_name(),
            "action.send",
        );
    }

    #[test]
    fn action_send_timeout_is_thirty_seconds() {
        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: crate::action::SendWireMessage {
                draft_id: "d".into(),
                from: "a@b".into(),
                to: vec!["c@d".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: None,
                body_html: String::new(),
                body_text: String::new(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: Vec::new(),
        };
        assert_eq!(
            RequestParams::ActionSend {
                request: Box::new(req),
            }
            .timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(30)),
        );
    }

    #[test]
    fn action_send_round_trips_from_method_params() {
        use crate::action::{SendAttachmentSource, SendWireAttachment, SendWireMessage};

        let req = SendWireRequest {
            send_id: PlanId::new_v7(),
            from_account_id: "acc-1".into(),
            message: SendWireMessage {
                draft_id: "draft-9".into(),
                from: "Alice <alice@example.com>".into(),
                to: vec!["bob@example.com".into()],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: Some("hello".into()),
                body_html: "<p>hi</p>".into(),
                body_text: "hi".into(),
                in_reply_to: None,
                references: None,
                thread_id: None,
            },
            attachments: vec![SendWireAttachment {
                source: SendAttachmentSource::StagingFile {
                    relative_path: "0.bin".into(),
                    content_hash: [3u8; 32],
                },
                size: 42,
                mime: "application/pdf".into(),
                filename: "x.pdf".into(),
                content_id: None,
            }],
        };
        let original = RequestParams::ActionSend {
            request: Box::new(req),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn sync_start_account_method_name_is_dotted() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(p.method_name(), "sync.start_account");
    }

    #[test]
    fn sync_start_account_timeout_is_five_seconds() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn sync_start_account_does_not_bypass_admission() {
        let p = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert!(!p.bypasses_admission());
    }

    #[test]
    fn sync_start_account_round_trips_from_method_params() {
        let original = RequestParams::SyncStartAccount {
            params: SyncStartAccountParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn sync_cancel_account_method_name_is_dotted() {
        let p = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(p.method_name(), "sync.cancel_account");
    }

    #[test]
    fn sync_cancel_account_timeout_is_five_seconds() {
        let p = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn sync_cancel_account_round_trips_from_method_params() {
        let original = RequestParams::SyncCancelAccount {
            params: SyncCancelAccountParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn calendar_set_visibility_method_name_is_dotted() {
        let p = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: true,
            },
        };
        assert_eq!(p.method_name(), "calendar.set_visibility");
    }

    #[test]
    fn calendar_set_visibility_timeout_is_five_seconds() {
        let p = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: true,
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn calendar_set_visibility_round_trips_from_method_params() {
        let original = RequestParams::CalendarSetVisibility {
            params: CalendarSetVisibilityParams {
                calendar_id: "cal-1".into(),
                visible: false,
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn thread_ui_state_set_method_name_is_dotted() {
        let p = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(true),
            },
        };
        assert_eq!(p.method_name(), "thread_ui_state.set");
    }

    #[test]
    fn thread_ui_state_set_timeout_is_five_seconds() {
        let p = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(true),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn thread_ui_state_set_round_trips_from_method_params() {
        let original = RequestParams::ThreadUiStateSet {
            params: ThreadUiStateSetParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
                attachments_collapsed: Some(false),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn settings_set_method_name_is_dotted() {
        let p = RequestParams::SettingsSet {
            params: SettingsSetParams {
                values: vec![crate::settings::SettingValue::ShowSyncStatus(true)],
            },
        };
        assert_eq!(p.method_name(), "settings.set");
    }

    #[test]
    fn settings_set_timeout_is_five_seconds() {
        let p = RequestParams::SettingsSet {
            params: SettingsSetParams {
                values: vec![crate::settings::SettingValue::ShowSyncStatus(true)],
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn settings_set_round_trips_from_method_params() {
        let original = RequestParams::SettingsSet {
            params: SettingsSetParams {
                values: vec![
                    crate::settings::SettingValue::ShowSyncStatus(true),
                    crate::settings::SettingValue::Theme("dark".into()),
                ],
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn boot_ready_round_trips_from_method_params() {
        let parsed = RequestParams::from_method_params("boot.ready", None).expect("parse");
        assert_eq!(parsed, RequestParams::BootReady);
        let parsed_null =
            RequestParams::from_method_params("boot.ready", Some(Value::Null)).expect("parse");
        assert_eq!(parsed_null, RequestParams::BootReady);
        assert!(
            RequestParams::from_method_params("boot.ready", Some(serde_json::json!({"x": 1})))
                .is_err()
        );
    }

    // -- Phase 6a: signature CRUD wire envelope -----------------------------

    fn sample_create_params() -> SignatureCreateParams {
        SignatureCreateParams {
            account_id: "acc-1".into(),
            name: "Work".into(),
            body_html: "<p>hi</p>".into(),
            body_text: Some("hi".into()),
            is_default: true,
            is_reply_default: false,
        }
    }

    #[test]
    fn signature_create_method_name_is_dotted() {
        let p = RequestParams::SignatureCreate {
            params: sample_create_params(),
        };
        assert_eq!(p.method_name(), "signature.create");
    }

    #[test]
    fn signature_create_timeout_is_five_seconds() {
        let p = RequestParams::SignatureCreate {
            params: sample_create_params(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn signature_create_round_trips_from_method_params() {
        let original = RequestParams::SignatureCreate {
            params: sample_create_params(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn signature_update_method_name_is_dotted() {
        let p = RequestParams::SignatureUpdate {
            params: SignatureUpdateParams {
                id: "sig-1".into(),
                name: Some("New".into()),
                body_html: None,
                body_text: None,
                is_default: None,
                is_reply_default: None,
            },
        };
        assert_eq!(p.method_name(), "signature.update");
    }

    #[test]
    fn signature_update_timeout_is_five_seconds() {
        let p = RequestParams::SignatureUpdate {
            params: SignatureUpdateParams {
                id: "sig-1".into(),
                name: None,
                body_html: None,
                body_text: None,
                is_default: Some(true),
                is_reply_default: None,
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn signature_update_round_trips_from_method_params() {
        let original = RequestParams::SignatureUpdate {
            params: SignatureUpdateParams {
                id: "sig-1".into(),
                name: Some("Renamed".into()),
                body_html: Some("<p>x</p>".into()),
                body_text: Some("x".into()),
                is_default: Some(false),
                is_reply_default: Some(true),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn signature_delete_method_name_is_dotted() {
        let p = RequestParams::SignatureDelete {
            params: SignatureDeleteParams { id: "sig-1".into() },
        };
        assert_eq!(p.method_name(), "signature.delete");
    }

    #[test]
    fn signature_delete_timeout_is_five_seconds() {
        let p = RequestParams::SignatureDelete {
            params: SignatureDeleteParams { id: "sig-1".into() },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn signature_delete_round_trips_from_method_params() {
        let original = RequestParams::SignatureDelete {
            params: SignatureDeleteParams { id: "sig-9".into() },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn signature_reorder_method_name_is_dotted() {
        let p = RequestParams::SignatureReorder {
            params: SignatureReorderParams {
                ordered_ids: vec!["a".into(), "b".into()],
            },
        };
        assert_eq!(p.method_name(), "signature.reorder");
    }

    #[test]
    fn signature_reorder_timeout_is_five_seconds() {
        let p = RequestParams::SignatureReorder {
            params: SignatureReorderParams {
                ordered_ids: Vec::new(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn signature_reorder_round_trips_from_method_params() {
        let original = RequestParams::SignatureReorder {
            params: SignatureReorderParams {
                ordered_ids: vec!["a".into(), "b".into(), "c".into()],
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6a: contact group wire envelope ----------------------------

    fn sample_group_save() -> ContactGroupSaveParams {
        ContactGroupSaveParams {
            id: "grp-1".into(),
            name: "Friends".into(),
            member_emails: vec!["a@example.com".into(), "b@example.com".into()],
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            member_count: 2,
        }
    }

    #[test]
    fn contacts_group_save_method_name_is_dotted() {
        let p = RequestParams::ContactsGroupSave {
            params: sample_group_save(),
        };
        assert_eq!(p.method_name(), "contacts.group_save");
    }

    #[test]
    fn contacts_group_save_timeout_is_five_seconds() {
        let p = RequestParams::ContactsGroupSave {
            params: sample_group_save(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn contacts_group_save_round_trips_from_method_params() {
        let original = RequestParams::ContactsGroupSave {
            params: sample_group_save(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn contacts_group_delete_method_name_is_dotted() {
        let p = RequestParams::ContactsGroupDelete {
            params: ContactGroupDeleteParams { id: "grp-1".into() },
        };
        assert_eq!(p.method_name(), "contacts.group_delete");
    }

    #[test]
    fn contacts_group_delete_timeout_is_five_seconds() {
        let p = RequestParams::ContactsGroupDelete {
            params: ContactGroupDeleteParams { id: "grp-1".into() },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn contacts_group_delete_round_trips_from_method_params() {
        let original = RequestParams::ContactsGroupDelete {
            params: ContactGroupDeleteParams { id: "grp-9".into() },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6d-A: contact save / delete with provider write-back ----

    fn sample_contact_save() -> ContactSaveParams {
        ContactSaveParams {
            id: "c-1".into(),
            email: "alice@example.com".into(),
            display_name: Some("Alice".into()),
            email2: None,
            phone: Some("+1-555-0100".into()),
            company: Some("Acme".into()),
            notes: None,
            account_id: Some("acc-1".into()),
            account_color: None,
            groups: Vec::new(),
            source: Some("google".into()),
            server_id: Some("people/c123".into()),
        }
    }

    #[test]
    fn contacts_contact_save_with_writeback_method_name_is_dotted() {
        let p = RequestParams::ContactsContactSaveWithWriteback {
            params: sample_contact_save(),
        };
        assert_eq!(p.method_name(), "contacts.contact_save_with_writeback");
    }

    #[test]
    fn contacts_contact_save_with_writeback_timeout_is_thirty_seconds() {
        let p = RequestParams::ContactsContactSaveWithWriteback {
            params: sample_contact_save(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(30)),
        );
    }

    #[test]
    fn contacts_contact_save_with_writeback_does_not_bypass_admission() {
        let p = RequestParams::ContactsContactSaveWithWriteback {
            params: sample_contact_save(),
        };
        assert!(!p.bypasses_admission());
    }

    #[test]
    fn contacts_contact_save_with_writeback_round_trips_from_method_params() {
        let original = RequestParams::ContactsContactSaveWithWriteback {
            params: sample_contact_save(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn contacts_contact_delete_method_name_is_dotted() {
        let p = RequestParams::ContactsContactDelete {
            params: ContactDeleteParams { id: "c-9".into() },
        };
        assert_eq!(p.method_name(), "contacts.contact_delete");
    }

    #[test]
    fn contacts_contact_delete_timeout_is_thirty_seconds() {
        let p = RequestParams::ContactsContactDelete {
            params: ContactDeleteParams { id: "c-9".into() },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(30)),
        );
    }

    #[test]
    fn contacts_contact_delete_round_trips_from_method_params() {
        let original = RequestParams::ContactsContactDelete {
            params: ContactDeleteParams { id: "c-9".into() },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6a: account update / reorder wire envelope -----------------

    fn sample_update() -> AccountUpdateParams {
        AccountUpdateParams {
            id: "acc-1".into(),
            account_name: Some("Work".into()),
            display_name: None,
            account_color: Some("#abcdef".into()),
            caldav_url: None,
            caldav_username: None,
            caldav_password: None,
            cache_attachments_enabled: None,
        }
    }

    #[test]
    fn account_update_method_name_is_dotted() {
        let p = RequestParams::AccountUpdate {
            params: sample_update(),
        };
        assert_eq!(p.method_name(), "account.update");
    }

    #[test]
    fn account_update_timeout_is_five_seconds() {
        let p = RequestParams::AccountUpdate {
            params: sample_update(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn account_update_round_trips_from_method_params() {
        let original = RequestParams::AccountUpdate {
            params: sample_update(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn account_reorder_method_name_is_dotted() {
        let p = RequestParams::AccountReorder {
            params: AccountReorderParams { orders: Vec::new() },
        };
        assert_eq!(p.method_name(), "account.reorder");
    }

    #[test]
    fn account_reorder_timeout_is_five_seconds() {
        let p = RequestParams::AccountReorder {
            params: AccountReorderParams { orders: Vec::new() },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    fn sample_create_for_envelope() -> AccountCreateParams {
        use crate::account::AccountCredentials;
        AccountCreateParams {
            email: "atle@example.com".into(),
            provider: "imap".into(),
            display_name: None,
            account_name: "Work".into(),
            account_color: String::new(),
            auth_method: "password".into(),
            credentials: AccountCredentials::Plaintext {
                access_token: None,
                refresh_token: None,
                imap_password: Some("secret".into()),
                smtp_password: None,
            },
            token_expires_at: None,
            oauth_provider: None,
            oauth_client_id: None,
            imap_host: Some("imap.example.com".into()),
            imap_port: Some(993),
            imap_security: Some("ssl".into()),
            imap_username: Some("atle".into()),
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            smtp_username: None,
            jmap_url: None,
            accept_invalid_certs: false,
        }
    }

    #[test]
    fn account_create_method_name_is_dotted() {
        let p = RequestParams::AccountCreate {
            params: Box::new(sample_create_for_envelope()),
        };
        assert_eq!(p.method_name(), "account.create");
    }

    #[test]
    fn account_create_timeout_is_five_seconds() {
        let p = RequestParams::AccountCreate {
            params: Box::new(sample_create_for_envelope()),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn account_create_round_trips_from_method_params() {
        let original = RequestParams::AccountCreate {
            params: Box::new(sample_create_for_envelope()),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn account_reorder_round_trips_from_method_params() {
        use crate::account::AccountReorderEntry;
        let original = RequestParams::AccountReorder {
            params: AccountReorderParams {
                orders: vec![
                    AccountReorderEntry {
                        account_id: "a".into(),
                        sort_order: 0,
                    },
                    AccountReorderEntry {
                        account_id: "b".into(),
                        sort_order: 1,
                    },
                ],
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6a-part-2: pinned-search CRUD wire envelope -----------------

    fn sample_create_or_update() -> PinnedSearchCreateOrUpdateParams {
        use crate::pinned_search::PinnedThreadRef;
        PinnedSearchCreateOrUpdateParams {
            query: "from:atle".into(),
            thread_ids: vec![PinnedThreadRef {
                thread_id: "t1".into(),
                account_id: "acc-1".into(),
            }],
            scope_account_id: Some("acc-1".into()),
        }
    }

    #[test]
    fn pinned_search_create_or_update_method_name_is_dotted() {
        let p = RequestParams::PinnedSearchCreateOrUpdate {
            params: sample_create_or_update(),
        };
        assert_eq!(p.method_name(), "pinned_search.create_or_update");
    }

    #[test]
    fn pinned_search_create_or_update_timeout_is_five_seconds() {
        let p = RequestParams::PinnedSearchCreateOrUpdate {
            params: sample_create_or_update(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn pinned_search_create_or_update_round_trips_from_method_params() {
        let original = RequestParams::PinnedSearchCreateOrUpdate {
            params: sample_create_or_update(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    fn sample_update_pinned() -> PinnedSearchUpdateParams {
        use crate::pinned_search::PinnedThreadRef;
        PinnedSearchUpdateParams {
            id: 7,
            query: "in:inbox".into(),
            thread_ids: vec![PinnedThreadRef {
                thread_id: "t9".into(),
                account_id: "acc-1".into(),
            }],
            scope_account_id: None,
        }
    }

    #[test]
    fn pinned_search_update_method_name_is_dotted() {
        let p = RequestParams::PinnedSearchUpdate {
            params: sample_update_pinned(),
        };
        assert_eq!(p.method_name(), "pinned_search.update");
    }

    #[test]
    fn pinned_search_update_timeout_is_five_seconds() {
        let p = RequestParams::PinnedSearchUpdate {
            params: sample_update_pinned(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn pinned_search_update_round_trips_from_method_params() {
        let original = RequestParams::PinnedSearchUpdate {
            params: sample_update_pinned(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn pinned_search_delete_method_name_is_dotted() {
        let p = RequestParams::PinnedSearchDelete {
            params: PinnedSearchDeleteParams { id: 3 },
        };
        assert_eq!(p.method_name(), "pinned_search.delete");
    }

    #[test]
    fn pinned_search_delete_timeout_is_five_seconds() {
        let p = RequestParams::PinnedSearchDelete {
            params: PinnedSearchDeleteParams { id: 3 },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn pinned_search_delete_round_trips_from_method_params() {
        let original = RequestParams::PinnedSearchDelete {
            params: PinnedSearchDeleteParams { id: 11 },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn pinned_search_delete_all_method_name_is_dotted() {
        let p = RequestParams::PinnedSearchDeleteAll {
            params: PinnedSearchDeleteAllParams,
        };
        assert_eq!(p.method_name(), "pinned_search.delete_all");
    }

    #[test]
    fn pinned_search_delete_all_timeout_is_five_seconds() {
        let p = RequestParams::PinnedSearchDeleteAll {
            params: PinnedSearchDeleteAllParams,
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn pinned_search_delete_all_round_trips_from_method_params() {
        let original = RequestParams::PinnedSearchDeleteAll {
            params: PinnedSearchDeleteAllParams,
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6a-part-2: smart-folder create wire envelope ----------------

    fn sample_smart_folder_create() -> SmartFolderCreateParams {
        SmartFolderCreateParams {
            name: "Unread VIPs".into(),
            query: "is:unread from:vip@example.com".into(),
        }
    }

    #[test]
    fn smart_folder_create_method_name_is_dotted() {
        let p = RequestParams::SmartFolderCreate {
            params: sample_smart_folder_create(),
        };
        assert_eq!(p.method_name(), "smart_folder.create");
    }

    #[test]
    fn smart_folder_create_timeout_is_five_seconds() {
        let p = RequestParams::SmartFolderCreate {
            params: sample_smart_folder_create(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn smart_folder_create_round_trips_from_method_params() {
        let original = RequestParams::SmartFolderCreate {
            params: sample_smart_folder_create(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn account_update_tokens_method_name_is_dotted() {
        let p = RequestParams::AccountUpdateTokens {
            params: Box::new(AccountUpdateTokensParams {
                account_id: "a".into(),
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                imap_password: None,
                smtp_password: None,
            }),
        };
        assert_eq!(p.method_name(), "account.update_tokens");
    }

    #[test]
    fn account_update_tokens_timeout_is_five_seconds() {
        let p = RequestParams::AccountUpdateTokens {
            params: Box::new(AccountUpdateTokensParams {
                account_id: "a".into(),
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                imap_password: None,
                smtp_password: None,
            }),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn account_update_tokens_round_trips_from_method_params() {
        let original = RequestParams::AccountUpdateTokens {
            params: Box::new(AccountUpdateTokensParams {
                account_id: "a".into(),
                access_token: Some(crate::redacted::RedactedString::new("at")),
                refresh_token: None,
                token_expires_at: Some(42),
                imap_password: None,
                smtp_password: None,
            }),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn account_delete_method_name_is_dotted() {
        let p = RequestParams::AccountDelete {
            params: AccountDeleteParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(p.method_name(), "account.delete");
    }

    #[test]
    fn account_delete_timeout_is_sixty_seconds() {
        let p = RequestParams::AccountDelete {
            params: AccountDeleteParams {
                account_id: "acc-1".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(60)),
        );
    }

    #[test]
    fn account_delete_round_trips_from_method_params() {
        let original = RequestParams::AccountDelete {
            params: AccountDeleteParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6a-part-2: encryption-key handle wire envelopes -------------

    #[test]
    fn read_bootstrap_snapshots_method_name_is_dotted() {
        let p = RequestParams::ReadBootstrapSnapshots {
            params: ReadBootstrapSnapshotsParams::default(),
        };
        assert_eq!(p.method_name(), "internal.read_bootstrap_snapshots");
    }

    #[test]
    fn read_bootstrap_snapshots_timeout_is_ten_seconds() {
        let p = RequestParams::ReadBootstrapSnapshots {
            params: ReadBootstrapSnapshotsParams::default(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(10)),
        );
    }

    #[test]
    fn read_bootstrap_snapshots_round_trips_from_method_params() {
        let original = RequestParams::ReadBootstrapSnapshots {
            params: ReadBootstrapSnapshotsParams::default(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn encrypt_for_storage_method_name_is_dotted() {
        let p = RequestParams::EncryptForStorage {
            params: EncryptForStorageParams {
                plaintext: crate::redacted::RedactedString::new("x"),
            },
        };
        assert_eq!(p.method_name(), "internal.encrypt_for_storage");
    }

    #[test]
    fn encrypt_for_storage_timeout_is_five_seconds() {
        let p = RequestParams::EncryptForStorage {
            params: EncryptForStorageParams {
                plaintext: crate::redacted::RedactedString::new("x"),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn encrypt_for_storage_round_trips_from_method_params() {
        let original = RequestParams::EncryptForStorage {
            params: EncryptForStorageParams {
                plaintext: crate::redacted::RedactedString::new("hunter2"),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn decrypt_for_storage_method_name_is_dotted() {
        let p = RequestParams::DecryptForStorage {
            params: DecryptForStorageParams {
                ciphertext: "AAAA:BBBB".into(),
            },
        };
        assert_eq!(p.method_name(), "internal.decrypt_for_storage");
    }

    #[test]
    fn decrypt_for_storage_timeout_is_five_seconds() {
        let p = RequestParams::DecryptForStorage {
            params: DecryptForStorageParams {
                ciphertext: "AAAA:BBBB".into(),
            },
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
    }

    #[test]
    fn decrypt_for_storage_round_trips_from_method_params() {
        let original = RequestParams::DecryptForStorage {
            params: DecryptForStorageParams {
                ciphertext: "AAAA:BBBB".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6b: attachment.fetch wire envelope -------------------------

    fn sample_attachment_fetch_params() -> AttachmentFetchParams {
        AttachmentFetchParams {
            account_id: "acct-1".into(),
            message_id: "msg-1".into(),
            attachment_id: "att-1".into(),
        }
    }

    #[test]
    fn attachment_fetch_method_name_is_dotted() {
        let p = RequestParams::AttachmentFetch {
            params: sample_attachment_fetch_params(),
        };
        assert_eq!(p.method_name(), "attachment.fetch");
    }

    #[test]
    fn attachment_fetch_timeout_is_sixty_seconds() {
        let p = RequestParams::AttachmentFetch {
            params: sample_attachment_fetch_params(),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(60)),
        );
    }

    #[test]
    fn attachment_fetch_round_trips_from_method_params() {
        let original = RequestParams::AttachmentFetch {
            params: sample_attachment_fetch_params(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    // -- Phase 6b: oauth.exchange_code wire envelope ----------------------

    fn sample_oauth_exchange_params() -> OauthExchangeCodeParams {
        OauthExchangeCodeParams {
            provider_id: "google".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            scopes: vec!["openid".into()],
            user_info_url: None,
            use_pkce: true,
            client_id: "client".into(),
            client_secret: None,
            redirect_uri: "http://127.0.0.1:54321/callback".into(),
            code: crate::redacted::RedactedString::new("authcode"),
            code_verifier: Some("pkce".into()),
            reauth_account_id: None,
        }
    }

    #[test]
    fn oauth_exchange_code_method_name_is_dotted() {
        let p = RequestParams::OauthExchangeCode {
            params: Box::new(sample_oauth_exchange_params()),
        };
        assert_eq!(p.method_name(), "oauth.exchange_code");
    }

    #[test]
    fn oauth_exchange_code_timeout_is_thirty_seconds() {
        let p = RequestParams::OauthExchangeCode {
            params: Box::new(sample_oauth_exchange_params()),
        };
        assert_eq!(
            p.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(30)),
        );
    }

    #[test]
    fn oauth_exchange_code_round_trips_from_method_params() {
        let original = RequestParams::OauthExchangeCode {
            params: Box::new(sample_oauth_exchange_params()),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn oauth_exchange_code_bypasses_admission() {
        // Locking the admission-bypass list. HealthPing and BootReady
        // were the originals; OauthExchangeCode joined in Phase 6b
        // because the OAuth round-trip cannot queue behind a 30 s
        // ActionSend or a long attachment.fetch.
        let p = RequestParams::OauthExchangeCode {
            params: Box::new(sample_oauth_exchange_params()),
        };
        assert!(p.bypasses_admission());

        // Sanity: a non-bypass variant doesn't.
        let q = RequestParams::DecryptForStorage {
            params: DecryptForStorageParams {
                ciphertext: "x".into(),
            },
        };
        assert!(!q.bypasses_admission());
    }

    #[test]
    fn test_seed_account_round_trips_from_method_params() {
        let original = RequestParams::TestSeedAccount {
            params: TestSeedAccountParams {
                email: Some("harness@example.test".into()),
                display_name: Some("Harness".into()),
                account_name: Some("Harness Account".into()),
                provider: Some("imap".into()),
                caldav_url: Some("http://127.0.0.1:12345".into()),
                caldav_username: Some("account-1".into()),
                caldav_password: Some("test-password".into()),
                auth_method: Some("oauth2".into()),
                access_token: Some("access-token".into()),
                refresh_token: Some("refresh-token".into()),
                token_expires_at: Some(1_800_000_000),
                oauth_provider: Some("oidc:saehrimnir".into()),
                oauth_client_id: Some("ratatoskr-harness".into()),
                oauth_token_url: Some("http://127.0.0.1:12345/oauth/token".into()),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(
            original.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_counter_read_round_trips_from_method_params() {
        let original = RequestParams::TestCounterRead {
            counter: "search.write".into(),
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(
            original.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
        assert_eq!(original.idempotency(), Idempotency::Idempotent);
    }

    #[test]
    fn test_crash_after_n_writes_round_trips_from_method_params() {
        let original = RequestParams::TestCrashAfterNWrites {
            params: TestCrashAfterNWritesParams {
                kind: "action.journal_write".into(),
                n: 2,
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(
            original.timeout(),
            RequestTimeoutKind::Finite(Duration::from_secs(5)),
        );
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_seed_thread_round_trips_from_method_params() {
        let original = RequestParams::TestSeedThread {
            params: TestSeedThreadParams {
                account_id: "acc-1".into(),
                thread_id: Some("thread-1".into()),
                message_id: Some("message-1".into()),
                subject: Some("M4".into()),
                label_ids: vec!["INBOX".into()],
                is_read: false,
                is_starred: true,
                is_pinned: false,
                is_muted: true,
                is_chat_thread: true,
                chat_email: Some("chat@example.test".into()),
                body_text: Some("Body text".into()),
                body_html: Some("<p>Body text</p>".into()),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.seed_thread");
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_seed_cached_attachment_round_trips_from_method_params() {
        let original = RequestParams::TestSeedCachedAttachment {
            params: TestSeedCachedAttachmentParams {
                account_id: "acc-1".into(),
                message_id: "message-1".into(),
                attachment_id: Some("attachment-1".into()),
                filename: Some("body.txt".into()),
                mime_type: Some("text/plain".into()),
                content: "attachment body".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.seed_cached_attachment");
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_seed_remote_attachment_round_trips_from_method_params() {
        let original = RequestParams::TestSeedRemoteAttachment {
            params: TestSeedRemoteAttachmentParams {
                account_id: "acc-1".into(),
                message_id: "message-1".into(),
                attachment_id: Some("attachment-1".into()),
                filename: Some("known-content.pdf".into()),
                mime_type: Some("application/pdf".into()),
                content_base64: "cGRmcmVhbGZpeHR1cmU=".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.seed_remote_attachment");
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_remove_cached_attachment_bytes_round_trips_from_method_params() {
        let original = RequestParams::TestRemoveCachedAttachmentBytes {
            params: TestRemoveCachedAttachmentBytesParams {
                relative_path: "attachment_cache/hash".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(
            original.method_name(),
            "test.remove_cached_attachment_bytes",
        );
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }

    #[test]
    fn test_thread_read_round_trips_from_method_params() {
        let original = RequestParams::TestThreadRead {
            params: TestThreadReadParams {
                account_id: "acc-1".into(),
                thread_id: "thread-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.thread_read");
        assert_eq!(original.idempotency(), Idempotency::Idempotent);
    }

    #[test]
    fn test_pending_ops_read_round_trips_from_method_params() {
        let original = RequestParams::TestPendingOpsRead {
            params: TestPendingOpsReadParams {
                account_id: Some("acc-1".into()),
                resource_id: Some("thread-1".into()),
                operation_type: Some("archive".into()),
                status: Some("pending".into()),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.pending_ops_read");
        assert_eq!(original.idempotency(), Idempotency::Idempotent);
    }

    #[test]
    fn test_start_sync_round_trips_from_method_params() {
        let original = RequestParams::TestStartSync {
            params: TestStartSyncParams {
                account_id: "acc-1".into(),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.start_sync");
        assert_eq!(original.idempotency(), Idempotency::Conditional);
    }

    #[test]
    fn test_query_db_state_round_trips_from_method_params() {
        let original = RequestParams::TestQueryDbState {
            params: TestQueryDbStateParams {
                account_id: Some("acc-1".into()),
                message_limit: Some(10),
                attachment_limit: Some(20),
                calendar_limit: Some(30),
                contact_limit: Some(40),
                contact_group_limit: Some(50),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.query_db_state");
        assert_eq!(original.idempotency(), Idempotency::Idempotent);
    }

    #[test]
    fn test_search_index_round_trips_from_method_params() {
        let original = RequestParams::TestSearchIndex {
            params: TestSearchIndexParams {
                query: "attachment phrase".into(),
                account_id: Some("acc-1".into()),
                limit: Some(5),
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.search_index");
        assert_eq!(original.idempotency(), Idempotency::Idempotent);
    }

    #[test]
    fn test_delay_next_write_round_trips_from_method_params() {
        let original = RequestParams::TestDelayNextWrite {
            params: TestDelayNextWriteParams {
                kind: "action.batch_execute".into(),
                millis: 250,
            },
        };
        let parsed = RequestParams::from_method_params(
            original.method_name(),
            Some(original.params_value()),
        )
        .expect("parse");
        assert_eq!(parsed, original);
        assert_eq!(original.method_name(), "test.delay_next_write");
        assert_eq!(original.idempotency(), Idempotency::Mutating);
    }
}
