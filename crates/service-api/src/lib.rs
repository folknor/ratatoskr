mod action;
mod boot;
mod client_notification;
mod error;
mod framing;
mod notification;
mod push;
mod redacted;
mod request;
mod response;
mod sync;
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
