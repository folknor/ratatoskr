//! Shared account-create helper.
//!
//! `account.create` (wire envelope from the UI's add-account flow)
//! and `oauth.exchange_code` (Phase 6b OAuth two-step) both
//! reach this helper after each has converted its own input shape
//! into `db::queries_extra::CreateAccountParams`. The helper owns
//! the DB write itself plus any post-create side effects we grow.
//!
//! The point of the indirection is not the DB call - that's a
//! one-liner `with_conn(create_account_sync)`. The point is "what
//! makes a fully-formed account" being defined once. If a future
//! release adds "every account needs a default folder set" or
//! "every account needs an initial signature" or similar, the
//! change lands here and both creation entry points pick it up.

use service_api::ServiceError;
use service_state::WriteDbState;

/// Insert a new account row and run any post-create side effects.
///
/// Returns the new account's id (the same string that
/// `create_account_sync` returns; today the UUID UI / OAuth-path
/// pre-generates and embeds in `params.email` is not the row id -
/// the row id comes from `create_account_sync`).
pub(crate) async fn create_account_inner(
    write_db: &WriteDbState,
    params: db::db::queries_extra::CreateAccountParams,
) -> Result<String, ServiceError> {
    let id = write_db
        .with_conn(move |conn| db::db::queries_extra::create_account_sync(conn, &params))
        .await
        .map_err(ServiceError::Internal)?;

    // Post-create side effects hook. Today this is a no-op; future
    // additions (default folder set, initial sync trigger, etc.)
    // land here so both creation entry points pick them up. Each
    // hook should be best-effort: a failure should log and proceed,
    // because the account row is committed and the user expects to
    // be able to see it even if a non-critical post-create step
    // wobbles.

    Ok(id)
}
