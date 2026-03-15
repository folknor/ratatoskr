use std::collections::HashMap;

use crate::db::DbState;

use super::super::client::GraphClient;
use super::super::types::{MESSAGE_SELECT, ODataCollection, REACTIONS_EXPAND};
use crate::sync::state as sync_state;

// ---------------------------------------------------------------------------
// Delta token management
// ---------------------------------------------------------------------------

/// Bootstrap a delta token for a folder by paging through the delta endpoint
/// until the server returns a `@odata.deltaLink` (no more `nextLink`).
///
/// Uses `$select=id` to minimize payload — we already have the messages from
/// the initial fetch.
pub(super) async fn bootstrap_delta_token(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<String, String> {
    let enc_folder_id = urlencoding::encode(folder_id);
    let me = client.api_path_prefix();
    let initial_url = format!(
        "{me}/mailFolders/{enc_folder_id}/messages/delta\
         ?$select={MESSAGE_SELECT}\
         &$expand=attachments($select=id,name,contentType,size,isInline,contentId,contentBytes),{REACTIONS_EXPAND}"
    );
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<serde_json::Value> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        if let Some(ref delta) = page.delta_link {
            return Ok(delta.clone());
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => {
                return Err(format!(
                    "Delta bootstrap for folder {folder_id} ended without a deltaLink"
                ));
            }
        }
    }
}

/// Save a delta token for a folder.
///
/// Routes to shared mailbox storage when the client is scoped to one.
pub(super) async fn save_delta_token(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    match client.mailbox_id() {
        Some(mailbox_id) => {
            sync_state::save_shared_mailbox_delta_token(
                db, account_id, mailbox_id, folder_id, delta_link,
            )
            .await
        }
        None => sync_state::save_graph_delta_token(db, account_id, folder_id, delta_link).await,
    }
}

/// Load all delta tokens for an account (or shared mailbox).
///
/// Routes to shared mailbox storage when the client is scoped to one.
pub(super) async fn load_delta_tokens(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    match client.mailbox_id() {
        Some(mailbox_id) => {
            sync_state::load_shared_mailbox_delta_tokens(db, account_id, mailbox_id).await
        }
        None => sync_state::load_graph_delta_tokens(db, account_id).await,
    }
}

/// Bootstrap a delta token for a folder using `$deltatoken=latest`.
///
/// This asks the server for a fresh delta token without fetching any existing
/// messages. Ideal for newly discovered folders during delta sync — we'll
/// pick up new messages starting from the next cycle.
pub(super) async fn bootstrap_delta_token_latest(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<String, String> {
    let enc_folder_id = urlencoding::encode(folder_id);
    let me = client.api_path_prefix();
    let url = format!(
        "{me}/mailFolders/{enc_folder_id}/messages/delta\
         ?$select={MESSAGE_SELECT}\
         &$expand=attachments($select=id,name,contentType,size,isInline,contentId,contentBytes),{REACTIONS_EXPAND}\
         &$deltatoken=latest"
    );
    let page: ODataCollection<serde_json::Value> = client.get_json(&url, db).await?;

    page.delta_link.ok_or_else(|| {
        format!("Delta bootstrap (latest) for folder {folder_id} returned no deltaLink")
    })
}

/// Delete a delta token for a folder that no longer exists.
///
/// Routes to shared mailbox storage when the client is scoped to one.
pub(super) async fn delete_delta_token(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    match client.mailbox_id() {
        Some(mailbox_id) => {
            sync_state::delete_shared_mailbox_delta_token(db, account_id, mailbox_id, folder_id)
                .await
        }
        None => sync_state::delete_graph_delta_token(db, account_id, folder_id).await,
    }
}
