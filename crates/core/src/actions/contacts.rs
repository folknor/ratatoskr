//! Contact write-back - save and delete through providers.
//!
//! JMAP, Google, and Graph are fully wired. CardDAV is a stub that returns
//! `LocalOnly` until vCard generation + PUT is implemented.

use super::context::ActionContext;
use super::log::MutationLog;
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
/// Display name is local-only - only phone, company, notes are pushed.
/// Returns `Success` if local + provider both succeeded, `LocalOnly` if
/// local succeeded but provider failed/stubbed, `Failed` if local failed.
pub async fn save_contact(ctx: &ActionContext, input: ContactSaveInput) -> ActionOutcome {
    let mlog = MutationLog::begin(
        "save_contact",
        input.account_id.as_deref().unwrap_or(""),
        &input.id,
    );

    // 1. Local DB save
    let db = ctx.db.clone();
    let inp = input.clone();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
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
        let outcome = ActionOutcome::Failed { error: e };
        mlog.emit(&outcome);
        return outcome;
    }

    // 2. Provider write-back for synced contacts
    let source = input.source.as_deref().unwrap_or("user");
    if source == "user" {
        let outcome = ActionOutcome::Success;
        mlog.emit(&outcome);
        return outcome;
    }

    // Need both account_id and server_id for provider dispatch
    let (Some(account_id), Some(server_id)) = (&input.account_id, &input.server_id) else {
        // Synced contact without account/server identity - can't dispatch.
        // Return LocalOnly, not Success: the local save succeeded but provider
        // write-back was impossible due to missing identity.
        let msg = format!(
            "Synced contact {} has source={source} but missing account_id or server_id",
            input.email
        );
        let outcome = ActionOutcome::LocalOnly {
            reason: ActionError::remote(msg),
            retryable: false,
        };
        mlog.emit(&outcome);
        return outcome;
    };

    let outcome = match dispatch_write_back(
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
        Err(e) => ActionOutcome::LocalOnly {
            reason: e,
            retryable: false,
        },
    };
    mlog.emit(&outcome);
    outcome
}

/// Delete a contact. For synced contacts with provider support, dispatches
/// provider delete first (provider-first for JMAP), then deletes locally.
/// For local contacts or providers without delete support, deletes locally
/// and returns `LocalOnly` for unimplemented providers.
pub async fn delete_contact(ctx: &ActionContext, contact_id: &str) -> ActionOutcome {
    let mut mlog = MutationLog::begin("delete_contact", "", contact_id);

    // 1. Look up contact identity from DB
    let db = ctx.db.clone();
    let cid = contact_id.to_string();
    let meta_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn
            .lock()
            .map_err(|e| ActionError::db(format!("db lock: {e}")))?;
        crate::db::queries_extra::action_helpers::get_contact_meta_by_id_sync(&conn, &cid)
            .map_err(ActionError::db)?
            .ok_or_else(|| ActionError::not_found(format!("contact {cid} not found")))
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r);

    let (source, server_id, account_id) = match meta_result {
        Ok(m) => m,
        Err(e) => {
            let outcome = ActionOutcome::Failed { error: e };
            mlog.emit(&outcome);
            return outcome;
        }
    };
    if let Some(ref aid) = account_id {
        mlog.set_account_id(aid);
    }
    if let Some(ref sid) = server_id {
        mlog.set_remote_id(sid);
    }

    let source_str = source.as_deref().unwrap_or("user");

    // 2. For synced contacts, attempt provider delete
    let mut provider_outcome = None;
    if source_str != "user" {
        if let (Some(aid), Some(sid)) = (account_id.as_deref(), server_id.as_deref()) {
            match dispatch_delete(ctx, source_str, aid, sid).await {
                Ok(()) => {}
                Err(e) => {
                    // Provider-first: don't delete locally if provider fails.
                    // CardDAV is still a stub - local-only until PUT is wired.
                    if matches!(source_str, "jmap" | "google" | "graph") {
                        let outcome = ActionOutcome::Failed { error: e };
                        mlog.emit(&outcome);
                        return outcome;
                    }
                    // Unimplemented providers → delete locally, report LocalOnly
                    provider_outcome = Some(e);
                }
            }
        }
    }

    // 3. Delete locally
    if let Err(e) = db_delete_contact(&ctx.db, contact_id.to_string()).await {
        let outcome = ActionOutcome::Failed {
            error: ActionError::db(e),
        };
        mlog.emit(&outcome);
        return outcome;
    }

    let outcome = match provider_outcome {
        Some(reason) => ActionOutcome::LocalOnly {
            reason,
            retryable: false,
        },
        None => ActionOutcome::Success,
    };
    mlog.emit(&outcome);
    outcome
}

// ── Provider dispatch ────────────────────────────────────

/// Dispatch contact write-back to the appropriate provider.
///
/// Takes (account_id, server_id) directly from ContactSaveInput - no
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
            let client =
                jmap::client::JmapClient::from_account(&ctx.db, account_id, &ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            jmap::contacts_sync::jmap_contacts_push_update(
                &client, server_id, phone, company, notes,
            )
            .await
            .map_err(ActionError::remote)
        }
        "google" => {
            // Build the update field mask - early return if nothing to update
            let mut update_fields = Vec::new();
            if phone.is_some() {
                update_fields.push("phoneNumbers");
            }
            if company.is_some() {
                update_fields.push("organizations");
            }
            if notes.is_some() {
                update_fields.push("biographies");
            }
            if update_fields.is_empty() {
                return Ok(()); // nothing to push
            }

            let client =
                gmail::client::GmailClient::from_account(&ctx.db, account_id, ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            let body = crate::contacts::sync_google::build_google_contact_update_body(
                phone, company, notes, "*", // etag "*" = skip optimistic locking
            );
            let field_mask = update_fields.join(",");
            let url = format!(
                "https://people.googleapis.com/v1/{server_id}:updateContact?updatePersonFields={field_mask}",
            );
            let _resp: serde_json::Value = client
                .patch_absolute(&url, &body, &ctx.db)
                .await
                .map_err(ActionError::remote)?;
            log::info!("[Google-Contacts] Updated contact {server_id}");
            Ok(())
        }
        "graph" => {
            let client =
                graph::client::GraphClient::from_account(&ctx.db, account_id, ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            let body =
                crate::contacts::sync_graph::build_graph_contact_update_body(phone, company, notes);
            client
                .patch(&format!("/me/contacts/{server_id}"), &body, &ctx.db)
                .await
                .map_err(ActionError::remote)?;
            log::info!("[Graph-Contacts] Updated contact {server_id}");
            Ok(())
        }
        "carddav" => Err(ActionError::not_implemented(
            "CardDAV contact write-back not implemented (PUT + vCard needed)",
        )),
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
            let client =
                jmap::client::JmapClient::from_account(&ctx.db, account_id, &ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            jmap_contact_delete(&client, server_id)
                .await
                .map_err(ActionError::remote)
        }
        "google" => {
            let client =
                gmail::client::GmailClient::from_account(&ctx.db, account_id, ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            let url = format!("https://people.googleapis.com/v1/{server_id}:deleteContact");
            client
                .delete_absolute(&url, &ctx.db)
                .await
                .map_err(ActionError::remote)?;
            log::info!("[Google-Contacts] Deleted contact {server_id}");
            Ok(())
        }
        "graph" => {
            let client =
                graph::client::GraphClient::from_account(&ctx.db, account_id, ctx.encryption_key)
                    .await
                    .map_err(ActionError::remote)?;
            client
                .delete(&format!("/me/contacts/{server_id}"), &ctx.db)
                .await
                .map_err(ActionError::remote)?;
            log::info!("[Graph-Contacts] Deleted contact {server_id}");
            Ok(())
        }
        "carddav" => Err(ActionError::not_implemented(
            "CardDAV contact delete not implemented",
        )),
        _ => Ok(()),
    }
}

/// Delete a contact via JMAP ContactCard/set destroy.
async fn jmap_contact_delete(
    client: &jmap::client::JmapClient,
    server_id: &str,
) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut set = jmap_client::contact_card::ContactCardSet::new(&req_account_id);
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
