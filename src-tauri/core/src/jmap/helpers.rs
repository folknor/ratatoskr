use super::client::JmapClient;
use super::mailbox_mapper::label_id_to_mailbox_id;

pub async fn query_thread_email_ids(
    client: &JmapClient,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    use jmap_client::email;

    let filter: jmap_client::core::query::Filter<email::query::Filter> =
        email::query::Filter::in_thread(thread_id).into();
    let result = client
        .inner()
        .email_query(Some(filter), None::<Vec<_>>)
        .await
        .map_err(|e| format!("Email/query inThread: {e}"))?;

    Ok(result.ids().to_vec())
}

/// Get the full mailbox list as (id, role, name) tuples.
///
/// Uses the TTL-cached mailbox list from `JmapClient` to avoid redundant
/// `Mailbox/get` API calls across consecutive thread actions.
pub async fn get_mailbox_list(
    client: &JmapClient,
) -> Result<Vec<super::client::MailboxListEntry>, String> {
    client.mailbox_list().await
}

/// Fetch the mailbox list directly from the server (bypassing cache).
/// Called by `JmapClient::mailbox_list()` on cache miss.
pub(super) async fn fetch_mailbox_list_from_server(
    client: &JmapClient,
) -> Result<Vec<super::client::MailboxListEntry>, String> {
    use jmap_client::mailbox::Role;

    let mailboxes = super::sync::fetch_all_mailboxes(client).await?;

    let mut result = Vec::new();
    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(super::sync::role_to_str(&role).to_string())
        };
        result.push((id.to_string(), role_str, name.to_string()));
    }
    Ok(result)
}

/// Get the first identity ID for email submission.
pub async fn get_first_identity_id(client: &jmap_client::client::Client) -> Result<String, String> {
    let mut request = client.build();
    request.get_identity();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Identity/get: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_identity().ok())
        .and_then(|mut r| r.take_list().into_iter().next().map(|mut i| i.take_id()))
        .ok_or_else(|| "No identity found for email submission".to_string())
}

/// Resolve a Gmail-style label ID to a JMAP mailbox ID.
pub async fn resolve_mailbox_id(client: &JmapClient, label_id: &str) -> Result<String, String> {
    let mailboxes = get_mailbox_list(client).await?;
    label_id_to_mailbox_id(label_id, &mailboxes)
        .ok_or_else(|| format!("Cannot resolve label \"{label_id}\" to JMAP mailbox"))
}
