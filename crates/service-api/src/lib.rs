mod action;
mod boot;
mod cal_action;
mod calendar;
mod client_notification;
mod error;
mod framing;
mod notification;
mod push;
mod redacted;
mod request;
mod response;
mod account;
mod attachment;
mod contacts;
mod internal;
mod oauth;
mod pinned_search;
mod settings;
mod signature;
mod smart_folder;
mod sync;
mod thread_ui_state;
mod version;

pub use action::{
    ActionCompleted, ActionPlanAck, ActionWireOperation, ActionWirePlan, JobStatusResponse,
    MarkChatReadAck, OperationId, OperationOutcome, OperationResult, PlanId, PlanSummary,
    RemoteFailure, SendAck, SendAttachmentSource, SendWireAttachment, SendWireMessage,
    SendWireRequest, SyncProgress, WireFolderId, WireJobStatus, WireMailOperation, WireTagId,
};
pub use client_notification::{ClientNotification, JsonRpcClientNotification};
pub use boot::{
    BootClassification, BootExitCode, BootPhase, BootPhaseKind, BootProgress, BootReadyResponse,
};
pub use cal_action::{
    CalendarActionCompleted, CalendarActionPlan, CalendarActionPlanAck,
    CalendarActionWireOperation, CalendarOperationOutcome, CalendarOperationResult,
    WireCalendarEventInput, WireCalendarOperation,
};
pub use calendar::{
    CalendarCancelAccountSyncParams, CalendarCancelAck, CalendarChanged, CalendarRunCompleted,
    CalendarRunId, CalendarSetVisibilityAck, CalendarSetVisibilityParams,
    CalendarStartAccountSyncParams, CalendarStartAck, CalendarSyncResult,
};
pub use account::{
    AccountCreateAck, AccountCreateParams, AccountCredentials, AccountDeleteAck,
    AccountDeleteParams, AccountReorderAck, AccountReorderEntry, AccountReorderParams,
    AccountUpdateAck, AccountUpdateParams, AccountUpdateTokensAck, AccountUpdateTokensParams,
};
pub use contacts::{
    ContactDeleteAck, ContactDeleteParams, ContactGroupDeleteAck, ContactGroupDeleteParams,
    ContactGroupSaveAck, ContactGroupSaveParams, ContactSaveAck, ContactSaveParams,
    ContactSaveWithWritebackAck, WritebackOutcome,
};
pub use internal::{
    DecryptForStorageAck, DecryptForStorageParams, EncryptForStorageAck, EncryptForStorageParams,
    ReadBootstrapSnapshotsAck, ReadBootstrapSnapshotsParams,
};
pub use attachment::{AttachmentFetchAck, AttachmentFetchParams};
pub use oauth::{OauthExchangeCodeAck, OauthExchangeCodeParams};
pub use pinned_search::{
    PinnedSearchCreateOrUpdateAck, PinnedSearchCreateOrUpdateParams, PinnedSearchDeleteAck,
    PinnedSearchDeleteAllAck, PinnedSearchDeleteAllParams, PinnedSearchDeleteParams,
    PinnedSearchUpdateAck, PinnedSearchUpdateParams, PinnedThreadRef,
};
pub use settings::{SettingValue, SettingsSetAck, SettingsSetParams};
pub use signature::{
    SignatureCreateAck, SignatureCreateParams, SignatureDeleteAck, SignatureDeleteParams,
    SignatureReorderAck, SignatureReorderParams, SignatureUpdateAck, SignatureUpdateParams,
};
pub use smart_folder::{SmartFolderCreateAck, SmartFolderCreateParams};
pub use thread_ui_state::{ThreadUiStateSetAck, ThreadUiStateSetParams};
pub use error::{JsonRpcErrorObject, ServiceError};
pub use framing::{
    BoundedLineReader, FrameError, JsonRpcErrorResponse, JsonRpcRequest, JsonRpcSuccessResponse,
    ParsedClientMessage, ParsedServiceMessage, RequestParseError, ServiceResponse,
    encode_message, parse_client_message, parse_service_message, write_message,
};
pub use notification::{CoalesceKey, Notification, NotificationClass, WithGeneration};
pub use push::PushEvent;
pub use redacted::{RedactedBytes, RedactedString};
pub use request::{RequestParams, RequestTimeoutKind};
pub use response::{HealthPingResponse, ShutdownResponse};
pub use sync::{
    IndexCommitted, SyncCancelAccountParams, SyncCancelAck, SyncCompleted, SyncResult, SyncRunId,
    SyncStartAccountParams, SyncStartAck,
};
pub use version::{MAX_FRAME_BYTES, PROTOCOL_VERSION};
