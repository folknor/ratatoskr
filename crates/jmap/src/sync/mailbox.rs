use std::collections::HashMap;

use jmap_client::mailbox::Role;

use super::super::client::JmapClient;
use super::super::mailbox_mapper::{MailboxInfo, map_mailbox_to_label};
use super::SyncCtx;

// ---------------------------------------------------------------------------
// Mailbox sync helpers
// ---------------------------------------------------------------------------

/// Fetch all mailboxes, persist as labels, return (mailbox_map, mailbox_data).
pub(super) async fn sync_mailboxes(
    ctx: &SyncCtx<'_>,
) -> Result<
    (
        HashMap<String, MailboxInfo>,
        Vec<(String, Option<String>, String)>,
    ),
    String,
> {
    let mailboxes = fetch_all_mailboxes(ctx.client).await?;

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

    // Second pass: build label rows with parent_label_id resolved
    let mut label_rows: Vec<(String, String, String, String, Option<String>)> = Vec::new();

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
        label_rows.push((
            mapping.label_id,
            aid.clone(),
            mapping.label_name,
            mapping.label_type.to_string(),
            parent_label_id,
        ));
    }

    // Also add pseudo-labels
    label_rows.push((
        "UNREAD".to_string(),
        aid.clone(),
        "Unread".to_string(),
        "system".to_string(),
        None,
    ));

    // Persist labels + categories to DB
    let category_rows: Vec<(String, String)> = label_rows
        .iter()
        .filter(|(_, _, _, lt, _)| lt == "user")
        .map(|(id, _, name, _, _)| (id.clone(), name.clone()))
        .collect();

    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (label_id, account_id, name, label_type, parent_label_id) in &label_rows {
                tx.execute(
                    "INSERT OR REPLACE INTO labels (id, account_id, name, type, parent_label_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![label_id, account_id, name, label_type, parent_label_id],
                )
                .map_err(|e| format!("upsert label: {e}"))?;
            }
            // Sync user mailboxes into categories table (no colors in JMAP)
            for (i, (provider_id, name)) in category_rows.iter().enumerate() {
                ratatoskr_db::db::queries::upsert_category(
                    &tx,
                    provider_id,
                    &aid,
                    name,
                    &ratatoskr_db::db::queries::CategoryColors {
                        preset: None,
                        bg: None,
                        fg: None,
                    },
                    provider_id,
                    i64::try_from(i).unwrap_or(0),
                    false,
                    ratatoskr_db::db::queries::CategorySortOnConflict::Keep,
                )?;
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
    // Fetch mailboxes to get the state string
    let inner = client.inner();
    let mut request = inner.build();
    request.get_mailbox();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox state: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_mailbox().ok())
        .map(|r| r.state().to_string())
        .ok_or_else(|| "No Mailbox/get response for state".to_string())
}

pub(super) async fn get_email_state(client: &JmapClient) -> Result<String, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let get_req = request.get_email();
    get_req.ids(std::iter::empty::<&str>());

    let response = request
        .send()
        .await
        .map_err(|e| format!("Email state: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_email().ok())
        .map(|r| r.state().to_string())
        .ok_or_else(|| "No Email/get response for state".to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fetch all mailboxes using the builder pattern (no filter = all mailboxes).
pub async fn fetch_all_mailboxes(
    client: &JmapClient,
) -> Result<Vec<jmap_client::mailbox::Mailbox<jmap_client::Get>>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    request.get_mailbox();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox/get: {e}"))?;

    Ok(response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_mailbox().ok())
        .map(|mut r| r.take_list())
        .unwrap_or_default())
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
