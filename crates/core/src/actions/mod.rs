//! Phase 2 task 6 shim - the action service execution surface lives in
//! the `service` crate now (`crates/service/src/actions/`). This file
//! re-exports the same public API so existing `rtsk::actions::*`
//! imports keep resolving while consumers migrate to using the IPC
//! path (`action.execute_plan`) added in tasks 9-10.
//!
//! The shim is transitional. Once the UI's `dispatch_plan` rewrite
//! (task 10) lands, no UI call site reaches into this module - the
//! action service is reached only via `service_client.execute_plan(...)`.
//! At that point this file (and the `core -> service` Cargo dep that
//! makes it work) can be deleted.

pub use service::actions::{
    ActionContext, ActionError, ActionOutcome, MailOperation, MutationLog, ProviderFolderMutation,
    RemoteFailureKind, SendAttachment, SendRequest, add_label, archive, batch_execute,
    create_folder, delete_draft, delete_folder, mark_read, move_to_folder, mute, permanent_delete,
    pin, remove_label, rename_folder, send_email, snooze, spam, star, trash, unsnooze,
};
// Submodule shims so existing `crate::actions::{pending,provider,contacts}::*`
// paths resolve unchanged: `core::chat` / `core::sync_dispatch` reach
// pending/provider; `app::handlers::contacts` reaches
// `actions::contacts::ContactSaveInput / save_contact / delete_contact`.
pub use service::actions::{contacts, pending, provider};
pub use common::typed_ids::{FolderId, TagId};
