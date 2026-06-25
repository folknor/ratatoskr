//! JMAP auxiliary sync passes preserved after the mail sync cutover.

use std::collections::{HashMap, HashSet};

use bifrost_jmap::mailbox::{MailboxGet, MailboxRights, Role};
use common::types::{FolderKind, MailProviderKind};
use db::db::ReadDbState;
use db::db::queries_extra::{FolderWriteRow, insert_folders_batch};
use service_state::WriteDbState;
use sync::state as sync_state;

use super::client::JmapClient;
use super::mailbox_mapper::{MailboxInfo, map_mailbox_to_folder};

pub struct AuxiliarySyncCtx<'a> {
    pub client: &'a JmapClient,
    pub account_id: &'a str,
    pub read_db: &'a ReadDbState,
    pub write_db: &'a WriteDbState,
}

struct MailboxFolderRow {
    id: String,
    account_id: String,
    name: String,
    folder_type: String,
    parent_folder_id: Option<String>,
    rights: Option<MailboxRights>,
    is_subscribed: Option<bool>,
}

pub async fn sync_mailboxes(
    ctx: &AuxiliarySyncCtx<'_>,
) -> Result<
    (
        HashMap<String, MailboxInfo>,
        Vec<(String, Option<String>, String)>,
    ),
    String,
> {
    let mailboxes = fetch_all_mailboxes_for(ctx.client, None).await?;
    let mut mailbox_map = HashMap::new();
    let mut mailbox_data = Vec::new();
    let mut jmap_id_to_folder_id: HashMap<String, String> = HashMap::new();

    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(role_to_str(&role))
        };
        let mapping = map_mailbox_to_folder(role_str, id, name)?;
        jmap_id_to_folder_id.insert(id.to_string(), mapping.folder_id);
    }

    let mut folder_rows = Vec::new();
    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(role_to_str(&role))
        };

        mailbox_map.insert(
            id.to_string(),
            MailboxInfo {
                role: role_str.map(String::from),
                name: name.to_string(),
            },
        );
        mailbox_data.push((id.to_string(), role_str.map(String::from), name.to_string()));

        let mapping = map_mailbox_to_folder(role_str, id, name)?;
        let parent_folder_id = mb
            .parent_id()
            .and_then(|pid| jmap_id_to_folder_id.get(pid))
            .cloned();
        folder_rows.push(MailboxFolderRow {
            id: mapping.folder_id,
            account_id: ctx.account_id.to_string(),
            name: mapping.folder_name,
            folder_type: mapping.folder_type.to_string(),
            parent_folder_id,
            rights: mb.my_rights().cloned(),
            is_subscribed: Some(mb.is_subscribed()),
        });
    }

    ctx.write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;
            let rows: Vec<FolderWriteRow> =
                folder_rows
                    .iter()
                    .map(|row| {
                        let (
                            r_read,
                            r_add,
                            r_remove,
                            r_seen,
                            r_kw,
                            r_child,
                            r_rename,
                            r_del,
                            r_submit,
                        ) = rights_to_ints(row.rights.as_ref());
                        FolderWriteRow {
                            id: row.id.clone(),
                            account_id: row.account_id.clone(),
                            name: row.name.clone(),
                            visible: None,
                            sort_order: None,
                            imap_folder_path: None,
                            imap_special_use: None,
                            namespace_type: None,
                            parent_id: row.parent_folder_id.clone(),
                            right_read: r_read,
                            right_add: r_add,
                            right_remove: r_remove,
                            right_set_seen: r_seen,
                            right_set_keywords: r_kw,
                            right_create_child: r_child,
                            right_rename: r_rename,
                            right_delete: r_del,
                            right_submit: r_submit,
                            is_subscribed: row.is_subscribed.map(i64::from),
                            is_undeletable: row.folder_type == "system",
                        }
                    })
                    .collect();
            insert_folders_batch(&tx, &rows)?;
            tx.commit().map_err(|e| format!("commit folders: {e}"))?;
            Ok(())
        })
        .await?;

    Ok((mailbox_map, mailbox_data))
}

pub async fn sync_mailbox_folder_map(
    ctx: &AuxiliarySyncCtx<'_>,
) -> Result<HashMap<String, FolderKind>, String> {
    let (_, mailbox_data) = sync_mailboxes(ctx).await?;
    let mut map = HashMap::new();
    for (mailbox_id, role, name) in mailbox_data {
        let mapping = map_mailbox_to_folder(role.as_deref(), &mailbox_id, &name)?;
        let folder = FolderKind::parse(&mapping.folder_id, MailProviderKind::Jmap)?;
        map.insert(mailbox_id, folder);
    }
    Ok(map)
}

pub async fn discover_shared_accounts(ctx: &AuxiliarySyncCtx<'_>) {
    let writer_pool = ctx.write_db.writer_pool();
    let session = ctx.client.inner().session();
    let mut session_shared_ids: Vec<(String, String)> = Vec::new();

    for jmap_account_id in session.accounts() {
        let Some(account) = session.account(jmap_account_id) else {
            continue;
        };
        if account.is_personal() {
            continue;
        }
        session_shared_ids.push((jmap_account_id.clone(), account.name().to_string()));
    }

    for (jmap_id, display_name) in &session_shared_ids {
        let dn = if display_name.is_empty() {
            None
        } else {
            Some(display_name.as_str())
        };
        if let Err(e) =
            sync_state::enable_shared_mailbox_sync(&writer_pool, ctx.account_id, jmap_id, dn).await
        {
            log::warn!(
                "[JMAP] Failed to enable shared account {jmap_id} for {}: {e}",
                ctx.account_id
            );
        }
    }

    let known_ids = match sync_state::get_all_shared_mailbox_ids(ctx.read_db, ctx.account_id).await
    {
        Ok(ids) => ids,
        Err(e) => {
            log::warn!(
                "[JMAP] Failed to load known shared mailboxes for {}: {e}",
                ctx.account_id
            );
            return;
        }
    };
    let session_id_set: HashSet<&str> = session_shared_ids
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();

    for known_id in &known_ids {
        if !session_id_set.contains(known_id.as_str()) {
            log::info!(
                "[JMAP] Shared account {known_id} no longer in Session for {} - disabling",
                ctx.account_id
            );
            if let Err(e) = sync_state::disable_shared_mailbox_sync_with_error(
                &writer_pool,
                ctx.account_id,
                known_id,
                "Access revoked - account no longer in JMAP Session",
            )
            .await
            {
                log::warn!("[JMAP] Failed to disable revoked shared account {known_id}: {e}");
            }
        }
    }

    if !session_shared_ids.is_empty() {
        log::info!(
            "[JMAP] Discovered {} shared account(s) for {}",
            session_shared_ids.len(),
            ctx.account_id
        );
    }
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
            log::info!("[JMAP] Mailbox sharing changed - re-running session discovery");
            discover_shared_accounts(ctx).await;
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

pub fn role_to_str(role: &Role) -> &'static str {
    match role {
        Role::Inbox => "inbox",
        Role::Archive => "archive",
        Role::Drafts => "drafts",
        Role::Sent => "sent",
        Role::Trash => "trash",
        Role::Junk => "junk",
        Role::Important => "important",
        _ => "other",
    }
}

#[allow(clippy::type_complexity)]
fn rights_to_ints(
    rights: Option<&MailboxRights>,
) -> (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
) {
    match rights {
        Some(r) => (
            Some(i64::from(r.may_read_items())),
            Some(i64::from(r.may_add_items())),
            Some(i64::from(r.may_remove_items())),
            Some(i64::from(r.may_set_seen())),
            Some(i64::from(r.may_set_keywords())),
            Some(i64::from(r.may_create_child())),
            Some(i64::from(r.may_rename())),
            Some(i64::from(r.may_delete())),
            Some(i64::from(r.may_submit())),
        ),
        None => (None, None, None, None, None, None, None, None, None),
    }
}
