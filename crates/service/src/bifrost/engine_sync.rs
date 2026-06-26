use db::db::ReadDbState;
use service_state::WriteDbState;

pub(crate) async fn prepare_jmap_mailboxes(
    client: &jmap::client::JmapClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<std::collections::HashMap<String, common::types::FolderKind>, String> {
    provider_sync::consumer_support::sync_jmap_mailbox_folder_map(
        client, account_id, read_db, write_db,
    )
    .await
}

pub(crate) async fn prepare_graph_folders(
    client: &graph::client::GraphClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<std::collections::HashMap<String, common::types::FolderKind>, String> {
    provider_sync::consumer_support::sync_graph_folder_map(client, account_id, read_db, write_db)
        .await
}

pub(crate) async fn prepare_gmail_labels(
    client: &gmail::client::GmailClient,
    account_id: &str,
    read_db: &ReadDbState,
    write_db: &WriteDbState,
) -> Result<std::collections::HashMap<String, common::types::FolderKind>, String> {
    provider_sync::consumer_support::sync_gmail_label_folder_map(
        client, account_id, read_db, write_db,
    )
    .await
}
