use std::collections::HashMap;

use jmap_client::email::EmailGet;
use jmap_client::mailbox::{MailboxGet, MailboxRights, Role};

use super::super::client::JmapClient;
use super::super::mailbox_mapper::{MailboxInfo, map_mailbox_to_label};
use super::SyncCtx;

// ---------------------------------------------------------------------------
// Mailbox sync helpers
// ---------------------------------------------------------------------------

/// A row to be persisted into the `labels` table, including optional rights.
struct LabelRow {
    label_id: String,
    account_id: String,
    label_name: String,
    label_type: String,
    parent_label_id: Option<String>,
    rights: Option<MailboxRights>,
    is_subscribed: Option<bool>,
}

/// Fetch all mailboxes, persist as labels, return (mailbox_map, mailbox_data).
pub(crate) async fn sync_mailboxes(
    ctx: &SyncCtx<'_>,
) -> Result<
    (
        HashMap<String, MailboxInfo>,
        Vec<(String, Option<String>, String)>,
    ),
    String,
> {
    let mailboxes = fetch_all_mailboxes_for(ctx.client, ctx.jmap_account_id.as_deref()).await?;

    let mut mailbox_map = HashMap::new();
    let mut mailbox_data = Vec::new();

    let aid = ctx.account_id.to_string();

    // First pass: build raw JMAP mailbox ID → label ID map for parent resolution
    let mut jmap_id_to_label_id: HashMap<String, String> = HashMap::new();
    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(role_to_str(&role))
        };
        let mapping = map_mailbox_to_label(role_str, id, name);
        jmap_id_to_label_id.insert(id.to_string(), mapping.label_id);
    }

    // Second pass: build label rows with parent_label_id resolved + rights
    let mut label_rows: Vec<LabelRow> = Vec::new();

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

        let mapping = map_mailbox_to_label(role_str, id, name);
        let parent_label_id = mb
            .parent_id()
            .and_then(|pid| jmap_id_to_label_id.get(pid))
            .cloned();
        label_rows.push(LabelRow {
            label_id: mapping.label_id,
            account_id: aid.clone(),
            label_name: mapping.label_name,
            label_type: mapping.label_type.to_string(),
            parent_label_id,
            rights: mb.my_rights().cloned(),
            is_subscribed: Some(mb.is_subscribed()),
        });
    }

    // Also add pseudo-labels
    label_rows.push(LabelRow {
        label_id: "UNREAD".to_string(),
        account_id: aid.clone(),
        label_name: "Unread".to_string(),
        label_type: "system".to_string(),
        parent_label_id: None,
        rights: None,
        is_subscribed: None,
    });

    // Persist labels to DB
    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for row in &label_rows {
                let (r_read, r_add, r_remove, r_seen, r_kw, r_child, r_rename, r_del, r_submit) =
                    rights_to_ints(row.rights.as_ref());
                tx.execute(
                    "INSERT INTO labels \
                     (id, account_id, name, type, parent_label_id, \
                      right_read, right_add, right_remove, right_set_seen, \
                      right_set_keywords, right_create_child, right_rename, \
                      right_delete, right_submit, is_subscribed) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                     ON CONFLICT(account_id, id) DO UPDATE SET \
                       name = excluded.name, \
                       type = excluded.type, \
                       parent_label_id = excluded.parent_label_id, \
                       right_read = excluded.right_read, \
                       right_add = excluded.right_add, \
                       right_remove = excluded.right_remove, \
                       right_set_seen = excluded.right_set_seen, \
                       right_set_keywords = excluded.right_set_keywords, \
                       right_create_child = excluded.right_create_child, \
                       right_rename = excluded.right_rename, \
                       right_delete = excluded.right_delete, \
                       right_submit = excluded.right_submit, \
                       is_subscribed = excluded.is_subscribed",
                    rusqlite::params![
                        row.label_id,
                        row.account_id,
                        row.label_name,
                        row.label_type,
                        row.parent_label_id,
                        r_read,
                        r_add,
                        r_remove,
                        r_seen,
                        r_kw,
                        r_child,
                        r_rename,
                        r_del,
                        r_submit,
                        row.is_subscribed,
                    ],
                )
                .map_err(|e| format!("upsert label: {e}"))?;
            }
            tx.commit().map_err(|e| format!("commit labels: {e}"))?;
            Ok(())
        })
        .await?;

    Ok((mailbox_map, mailbox_data))
}

/// Handle Mailbox/changes during delta sync.
pub(super) async fn sync_mailbox_changes(
    ctx: &SyncCtx<'_>,
    since_state: &str,
) -> Result<(), String> {
    let inner = ctx.client.inner();
    let result = inner.mailbox_changes(since_state, 500).await;

    match result {
        Ok(changes) => {
            let new_state = changes.new_state().to_string();
            if new_state != since_state {
                // State changed -- re-sync all mailboxes
                sync_mailboxes(ctx).await?;
                super::save_sync_state(ctx.db, ctx.account_id, "Mailbox", &new_state).await?;
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("cannotCalculateChanges") {
                // Full mailbox refresh
                let (_, _) = sync_mailboxes(ctx).await?;
                let new_state = get_mailbox_state(ctx.client).await?;
                super::save_sync_state(ctx.db, ctx.account_id, "Mailbox", &new_state).await?;
            } else {
                return Err(format!("Mailbox/changes: {msg}"));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// JMAP state getters
// ---------------------------------------------------------------------------

pub(super) async fn get_mailbox_state(client: &JmapClient) -> Result<String, String> {
    get_mailbox_state_for(client, None).await
}

pub(crate) async fn get_mailbox_state_for(
    client: &JmapClient,
    jmap_account_id: Option<&str>,
) -> Result<String, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = jmap_account_id
        .map(String::from)
        .unwrap_or_else(|| request.default_account_id().to_string());
    let get = MailboxGet::new(&account_id);
    let handle = request
        .call(get)
        .map_err(|e| format!("Mailbox state: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox state: {e}"))?;

    response
        .get(&handle)
        .map(|r| r.state().to_string())
        .map_err(|e| format!("Mailbox state: {e}"))
}

pub(super) async fn get_email_state(client: &JmapClient) -> Result<String, String> {
    get_email_state_for(client, None).await
}

pub(crate) async fn get_email_state_for(
    client: &JmapClient,
    jmap_account_id: Option<&str>,
) -> Result<String, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = jmap_account_id
        .map(String::from)
        .unwrap_or_else(|| request.default_account_id().to_string());
    let mut get = EmailGet::new(&account_id);
    get.ids(std::iter::empty::<&str>());
    let handle = request.call(get).map_err(|e| format!("Email state: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Email state: {e}"))?;

    response
        .get(&handle)
        .map(|r| r.state().to_string())
        .map_err(|e| format!("Email state: {e}"))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fetch all mailboxes using the builder pattern (no filter = all mailboxes).
pub async fn fetch_all_mailboxes(
    client: &JmapClient,
) -> Result<Vec<jmap_client::mailbox::Mailbox<jmap_client::Get>>, String> {
    fetch_all_mailboxes_for(client, None).await
}

/// Fetch all mailboxes for a specific JMAP account.
pub async fn fetch_all_mailboxes_for(
    client: &JmapClient,
    jmap_account_id: Option<&str>,
) -> Result<Vec<jmap_client::mailbox::Mailbox<jmap_client::Get>>, String> {
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

pub(crate) fn role_to_str(role: &jmap_client::mailbox::Role) -> &'static str {
    use jmap_client::mailbox::Role;
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

/// Convert `MailboxRights` to 9 `Option<i64>` values for SQL parameters.
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
