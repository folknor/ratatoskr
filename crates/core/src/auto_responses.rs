//! Local auto-response status scaffolding.
//!
//! Server-side vacation settings are capability-dispatched through the service
//! bifrost account-settings surface. This module intentionally retains only the
//! local status-bar read until a settings product caller is introduced.

/// Check whether any account has an active auto-response.
pub fn any_auto_response_active(conn: &crate::db::ReadConn<'_>) -> Result<bool, String> {
    crate::db::queries_extra::auto_responses::any_auto_response_active_sync(conn)
}
