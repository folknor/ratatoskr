use bifrost_jmap::mailbox::MailboxGet;

use crate::client::JmapClient;

/// Fetch all mailboxes using the builder pattern (no filter = all mailboxes).
pub async fn fetch_all_mailboxes(
    client: &JmapClient,
) -> Result<Vec<bifrost_jmap::mailbox::Mailbox<bifrost_jmap::Get>>, String> {
    fetch_all_mailboxes_for(client, None).await
}

/// Fetch all mailboxes for a specific JMAP account.
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

pub fn role_to_str(role: &bifrost_jmap::mailbox::Role) -> &'static str {
    use bifrost_jmap::mailbox::Role;
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
