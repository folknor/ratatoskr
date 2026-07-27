//! JMAP auxiliary sync passes preserved after the mail sync cutover.

use bifrost_jmap::mailbox::MailboxGet;
use db::db::ReadDbState;
use service_state::WriteDbState;
use sync::state as sync_state;

use super::client::JmapClient;

pub struct AuxiliarySyncCtx<'a> {
    pub client: &'a JmapClient,
    pub account_id: &'a str,
    pub read_db: &'a ReadDbState,
    pub write_db: &'a WriteDbState,
}

pub async fn resolve_shared_account_identities(ctx: &AuxiliarySyncCtx<'_>) {
    let writer_pool = ctx.write_db.writer_pool();
    let inner = ctx.client.inner();
    let session = inner.session();

    if !session.has_capability("urn:ietf:params:jmap:principals") {
        return;
    }

    let principals_account_id = session
        .principals_capabilities()
        .and_then(|c| c.account_id_for_principal())
        .map(String::from);

    for jmap_account_id in session.accounts() {
        let Some(account) = session.account(jmap_account_id) else {
            continue;
        };
        if account.is_personal() {
            continue;
        }

        match sync_state::get_shared_mailbox_email(ctx.read_db, ctx.account_id, jmap_account_id)
            .await
        {
            Ok(Some(_)) => continue,
            Ok(None) => {}
            Err(e) => {
                log::debug!("[JMAP] Failed to check shared mailbox email: {e}");
                continue;
            }
        }

        let owner_principal_id = account
            .capability("urn:ietf:params:jmap:principals:owner")
            .and_then(|cap| match cap {
                bifrost_jmap::core::session::Capabilities::PrincipalsOwner(owner) => {
                    owner.principal_id().map(String::from)
                }
                _ => None,
            });

        let Some(principal_id) = owner_principal_id else {
            let account_name = account.name();
            if account_name.contains('@') {
                if let Err(e) = sync_state::set_shared_mailbox_email(
                    &writer_pool,
                    ctx.account_id,
                    jmap_account_id,
                    account_name,
                )
                .await
                {
                    log::debug!("[JMAP] Failed to set shared mailbox email from name: {e}");
                }
                log::info!(
                    "[JMAP] Resolved shared account {jmap_account_id} email from name: {account_name}"
                );
            }
            continue;
        };

        let email = match fetch_principal_email(
            ctx.client,
            principals_account_id.as_deref(),
            &principal_id,
        )
        .await
        {
            Ok(Some(email)) => email,
            Ok(None) => {
                log::debug!(
                    "[JMAP] Principal {principal_id} has no email for shared account {jmap_account_id}"
                );
                continue;
            }
            Err(e) => {
                log::debug!("[JMAP] Failed to fetch principal {principal_id}: {e}");
                continue;
            }
        };

        if let Err(e) = sync_state::set_shared_mailbox_email(
            &writer_pool,
            ctx.account_id,
            jmap_account_id,
            &email,
        )
        .await
        {
            log::warn!("[JMAP] Failed to cache shared account email: {e}");
        } else {
            log::info!("[JMAP] Resolved shared account {jmap_account_id} email: {email}");
        }
    }
}

pub async fn poll_share_notifications(ctx: &AuxiliarySyncCtx<'_>) {
    let writer_pool = ctx.write_db.writer_pool();
    let inner = ctx.client.inner();
    let session = inner.session();

    if !session.has_capability("urn:ietf:params:jmap:principals") {
        return;
    }

    let since_state =
        match sync_state::load_jmap_sync_state(ctx.read_db, ctx.account_id, "ShareNotification")
            .await
        {
            Ok(state) => state,
            Err(e) => {
                log::warn!("[JMAP] Failed to load ShareNotification state: {e}");
                return;
            }
        };

    if since_state.is_none() {
        match get_share_notification_state(ctx.client).await {
            Ok(state) => {
                if let Err(e) = sync_state::save_jmap_sync_state(
                    &writer_pool,
                    ctx.account_id,
                    "ShareNotification",
                    &state,
                )
                .await
                {
                    log::warn!("[JMAP] Failed to save initial ShareNotification state: {e}");
                }
            }
            Err(e) => {
                log::debug!("[JMAP] ShareNotification/get not available: {e}");
            }
        }
        return;
    }

    let since = since_state.expect("checked above");
    let changes = match inner.share_notification_changes(&since, 500).await {
        Ok(c) => c,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("cannotCalculateChanges") {
                if let Ok(state) = get_share_notification_state(ctx.client).await {
                    let _ = sync_state::save_jmap_sync_state(
                        &writer_pool,
                        ctx.account_id,
                        "ShareNotification",
                        &state,
                    )
                    .await;
                }
            } else {
                log::warn!("[JMAP] ShareNotification/changes failed: {msg}");
            }
            return;
        }
    };

    let new_state = changes.new_state().to_string();
    let created = changes.created();

    if !created.is_empty() {
        log::info!(
            "[JMAP] {} new ShareNotification(s) for {}",
            created.len(),
            ctx.account_id
        );

        let mut has_mailbox_change = false;
        for notif_id in created {
            match inner
                .share_notification_get(
                    notif_id,
                    None::<Vec<bifrost_jmap::share_notification::Property>>,
                )
                .await
            {
                Ok(Some(notif)) => {
                    let obj_type = notif.object_type().unwrap_or("unknown");
                    let obj_name = notif.name().unwrap_or("(unnamed)");
                    let changer = notif
                        .changed_by()
                        .and_then(|cb| cb.name().or(cb.email()))
                        .unwrap_or("unknown");

                    if notif.new_rights().is_some() {
                        log::info!(
                            "[JMAP] Share granted: {changer} shared {obj_type} \"{obj_name}\""
                        );
                    } else {
                        log::info!(
                            "[JMAP] Share revoked: {changer} revoked access to {obj_type} \"{obj_name}\""
                        );
                    }

                    if obj_type == "Mailbox" {
                        has_mailbox_change = true;
                    }

                    if let Err(e) = inner.share_notification_destroy(notif_id).await {
                        log::debug!("[JMAP] Failed to destroy ShareNotification {notif_id}: {e}");
                    }
                }
                Ok(None) => {
                    log::debug!(
                        "[JMAP] ShareNotification {notif_id} not found (already destroyed?)"
                    );
                }
                Err(e) => {
                    log::debug!("[JMAP] Failed to fetch ShareNotification {notif_id}: {e}");
                }
            }
        }

        if has_mailbox_change {
            // The resident container snapshot owns foreign-account discovery
            // and reconciliation. Its next refresh observes the changed
            // session without a JMAP-only registry side path here.
            log::info!("[JMAP] Mailbox sharing changed; container refresh will reconcile it");
        }
    }

    if let Err(e) = sync_state::save_jmap_sync_state(
        &writer_pool,
        ctx.account_id,
        "ShareNotification",
        &new_state,
    )
    .await
    {
        log::warn!("[JMAP] Failed to save ShareNotification state: {e}");
    }
}

pub async fn fetch_all_mailboxes_for(
    client: &JmapClient,
    jmap_account_id: Option<&str>,
) -> Result<Vec<bifrost_jmap::mailbox::Mailbox<bifrost_jmap::Get>>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = jmap_account_id
        .map(String::from)
        .unwrap_or_else(|| request.default_account_id().to_string());
    let get = MailboxGet::new(&account_id);
    let handle = request.call(get).map_err(|e| format!("Mailbox/get: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox/get: {e}"))?;

    response
        .get(&handle)
        .map(|mut r| r.take_list())
        .map_err(|e| format!("Mailbox/get: {e}"))
}

async fn fetch_principal_email(
    client: &JmapClient,
    principals_account_id: Option<&str>,
    principal_id: &str,
) -> Result<Option<String>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = principals_account_id
        .map(String::from)
        .unwrap_or_else(|| request.default_account_id().to_string());

    let mut get = bifrost_jmap::principal::PrincipalGet::new(&account_id);
    get.ids([principal_id]);
    get.properties([
        bifrost_jmap::principal::Property::Email,
        bifrost_jmap::principal::Property::Name,
    ]);

    let handle = request
        .call(get)
        .map_err(|e| format!("Principal/get: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Principal/get: {e}"))?;

    let principal = response
        .get(&handle)
        .map(|mut r| r.take_list().pop())
        .map_err(|e| format!("Principal/get: {e}"))?;

    Ok(principal.and_then(|p| p.email().map(String::from)))
}

async fn get_share_notification_state(client: &JmapClient) -> Result<String, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let get = bifrost_jmap::share_notification::ShareNotificationGet::new(&account_id);
    let handle = request
        .call(get)
        .map_err(|e| format!("ShareNotification state: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("ShareNotification state: {e}"))?;
    response
        .get(&handle)
        .map(|r| r.state().to_string())
        .map_err(|e| format!("ShareNotification state: {e}"))
}
