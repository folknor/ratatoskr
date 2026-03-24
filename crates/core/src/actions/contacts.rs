//! Contact write-back — save and delete through providers.
//!
//! JMAP is fully wired. Google, Graph, and CardDAV are stubs that return
//! `LocalOnly` with a descriptive reason until their HTTP calls are implemented.

use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use crate::db::queries_extra::contacts::{db_delete_contact, db_upsert_contact_full};

// ── Public types ─────────────────────────────────────────

/// Provider-agnostic input for contact save.
#[derive(Debug, Clone)]
pub struct ContactSaveInput {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    /// Used for local save and for scoping provider lookups.
    pub account_id: Option<String>,
    /// Provider source: "user", "google", "graph", "jmap", "carddav".
    /// Determines whether and where write-back is dispatched.
    pub source: Option<String>,
    /// Provider-assigned server ID for synced contacts. Carried from the
    /// DB through the editor so write-back dispatch avoids ambiguous
    /// email-based lookups. None for local ("user") contacts.
    pub server_id: Option<String>,
}

// ── Action functions ─────────────────────────────────────

/// Save a contact locally, then dispatch write-back to the provider.
///
/// Display name is local-only — only phone, company, notes are pushed.
/// Returns `Success` if local + provider both succeeded, `LocalOnly` if
/// local succeeded but provider failed/stubbed, `Failed` if local failed.
pub async fn save_contact(ctx: &ActionContext, input: ContactSaveInput) -> ActionOutcome {
    // 1. Local DB save
    let db = ctx.db.clone();
    let inp = input.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        let source = inp.source.as_deref().unwrap_or("user");
        db_upsert_contact_full(
            &conn,
            &inp.id,
            &inp.email,
            inp.display_name.as_deref(),
            inp.email2.as_deref(),
            inp.phone.as_deref(),
            inp.company.as_deref(),
            inp.notes.as_deref(),
            inp.account_id.as_deref(),
            source,
        )
        .map_err(ActionError::db)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    if let Err(e) = local_result {
        return ActionOutcome::Failed { error: e };
    }

    // 2. Provider write-back for synced contacts
    let source = input.source.as_deref().unwrap_or("user");
    if source == "user" {
        return ActionOutcome::Success;
    }

    // Need both account_id and server_id for provider dispatch
    let (Some(account_id), Some(server_id)) = (&input.account_id, &input.server_id) else {
        // Synced contact without account/server identity — can't dispatch.
        // Return LocalOnly, not Success: the local save succeeded but provider
        // write-back was impossible due to missing identity.
        let msg = format!(
            "Synced contact {} has source={source} but missing account_id or server_id",
            input.email
        );
        log::warn!("{msg}");
        return ActionOutcome::LocalOnly { reason: ActionError::remote(msg), retryable: false };
    };

    match dispatch_write_back(
        ctx,
        source,
        account_id,
        server_id,
        input.phone.as_deref(),
        input.company.as_deref(),
        input.notes.as_deref(),
    )
    .await
    {
        Ok(()) => ActionOutcome::Success,
        Err(e) => {
            log::warn!("Contact write-back failed for {}: {e}", input.email);
            ActionOutcome::LocalOnly { reason: e, retryable: false }
        }
    }
}

/// Delete a contact. For synced contacts with provider support, dispatches
/// provider delete first (provider-first for JMAP), then deletes locally.
/// For local contacts or providers without delete support, deletes locally
/// and returns `LocalOnly` for unimplemented providers.
pub async fn delete_contact(ctx: &ActionContext, contact_id: &str) -> ActionOutcome {
    // 1. Look up contact identity from DB
    let db = ctx.db.clone();
    let cid = contact_id.to_string();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        conn.query_row(
            "SELECT source, server_id, account_id FROM contacts WHERE id = ?1",
            rusqlite::params![cid],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                ActionError::not_found(format!("contact lookup: {e}"))
            }
            _ => ActionError::db(format!("contact lookup: {e}")),
        })
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (source, server_id, account_id) = match meta_result {
        Ok(m) => m,
        Err(e) => return ActionOutcome::Failed { error: e },
    };

    let source_str = source.as_deref().unwrap_or("user");

    // 2. For synced contacts, attempt provider delete
    let mut provider_outcome = None;
    if source_str != "user" {
        if let (Some(aid), Some(sid)) = (account_id.as_deref(), server_id.as_deref()) {
            match dispatch_delete(ctx, source_str, aid, sid).await {
                Ok(()) => {}
                Err(e) => {
                    // JMAP failure → don't delete locally (provider-first).
                    // When Google/Graph/CardDAV HTTP delete is wired, extend
                    // this check to include them (all synced providers should
                    // be provider-first once their HTTP calls are real).
                    if source_str == "jmap" {
                        return ActionOutcome::Failed { error: e };
                    }
                    // Unimplemented providers → delete locally, report LocalOnly
                    provider_outcome = Some(e);
                }
            }
        }
    }

    // 3. Delete locally
    if let Err(e) = db_delete_contact(&ctx.db, contact_id.to_string()).await {
        return ActionOutcome::Failed { error: ActionError::db(e) };
    }

    match provider_outcome {
        Some(reason) => ActionOutcome::LocalOnly { reason, retryable: false },
        None => ActionOutcome::Success,
    }
}

// ── Provider dispatch ────────────────────────────────────

/// Dispatch contact write-back to the appropriate provider.
///
/// Takes (account_id, server_id) directly from ContactSaveInput — no
/// email-based server-info lookup, eliminating cross-account ambiguity.
async fn dispatch_write_back(
    ctx: &ActionContext,
    source: &str,
    account_id: &str,
    server_id: &str,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> Result<(), ActionError> {
    match source {
        "jmap" => {
            let client = ratatoskr_jmap::client::JmapClient::from_account(
                &ctx.db,
                account_id,
                &ctx.encryption_key,
            )
            .await
            .map_err(ActionError::remote)?;
            ratatoskr_jmap::contacts_sync::jmap_contacts_push_update(
                &client, server_id, phone, company, notes,
            )
            .await
            .map_err(ActionError::remote)
        }
        "google" => {
            // Scaffolding ready (build_google_contact_update_body,
            // get_google_contact_server_info). HTTP PATCH not wired.
            Err(ActionError::not_implemented("Google contact write-back not yet wired to HTTP"))
        }
        "graph" => Err(ActionError::not_implemented("Graph contact write-back not yet wired to HTTP")),
        "carddav" => Err(ActionError::not_implemented("CardDAV contact write-back not implemented (PUT + vCard needed)")),
        "user" => Ok(()),
        other => {
            log::warn!("Unknown contact source for write-back: {other}");
            Ok(())
        }
    }
}

/// Dispatch contact delete to the appropriate provider.
async fn dispatch_delete(
    ctx: &ActionContext,
    source: &str,
    account_id: &str,
    server_id: &str,
) -> Result<(), ActionError> {
    match source {
        "jmap" => {
            let client = ratatoskr_jmap::client::JmapClient::from_account(
                &ctx.db,
                account_id,
                &ctx.encryption_key,
            )
            .await
            .map_err(ActionError::remote)?;
            jmap_contact_delete(&client, server_id)
                .await
                .map_err(ActionError::remote)
        }
        "google" => Err(ActionError::not_implemented("Google contact delete not yet wired to HTTP")),
        "graph" => Err(ActionError::not_implemented("Graph contact delete not yet wired to HTTP")),
        "carddav" => Err(ActionError::not_implemented("CardDAV contact delete not implemented")),
        _ => Ok(()),
    }
}

/// Delete a contact via JMAP ContactCard/set destroy.
async fn jmap_contact_delete(
    client: &ratatoskr_jmap::client::JmapClient,
    server_id: &str,
) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut set =
        jmap_client::contact_card::ContactCardSet::new(&req_account_id);
    set.destroy([server_id]);
    let handle = request
        .call(set)
        .map_err(|e| format!("ContactCard/set (destroy): {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("ContactCard/set (destroy): {e}"))?;
    response
        .get(&handle)
        .map_err(|e| format!("ContactCard/set (destroy): {e}"))?;
    log::info!("[JMAP-Contacts] Deleted contact {server_id}");
    Ok(())
}
