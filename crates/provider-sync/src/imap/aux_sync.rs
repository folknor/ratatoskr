use std::collections::HashMap;

use common::types::FolderKind;
use db::db::queries_extra::{FolderWriteRow, insert_folders_batch};
use imap::folder_mapper::{get_syncable_folders, map_folder_to_folder};
use imap::types::ImapFolder;
use service_state::WriteDbState;

pub async fn sync_imap_folder_map(
    session: &mut imap::connection::ImapSession,
    account_id: &str,
    write_db: &WriteDbState,
) -> Result<HashMap<String, FolderKind>, String> {
    let folders = imap::client::list_folders(session).await?;
    let syncable = get_syncable_folders(&folders);
    let folder_map = folder_map(&syncable)?;
    let rows = folder_rows(account_id, &syncable)?;
    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|error| format!("begin IMAP folder tx: {error}"))?;
            insert_folders_batch(&tx, &rows)?;
            tx.commit()
                .map_err(|error| format!("commit IMAP folder tx: {error}"))?;
            Ok(())
        })
        .await?;
    Ok(folder_map)
}

/// Probe IMAP keyword (custom-flag) capability and persist the account-level
/// `supports_keywords` flag.
///
/// Deliberately runs on EVERY kick with no `initial_sync_completed` gate,
/// unlike the JMAP/Graph/Gmail auxiliary passes (whose heavy delta work is
/// initial-vs-delta gated). The deviation is required for correctness:
/// keyword capability is advertised per-mailbox in PERMANENTFLAGS, only
/// readable by SELECTing the mailbox, and the account flag is a conservative
/// AND across all folders (the server supports custom keywords only if every
/// mailbox does). The folder set is re-LISTed every kick and a new mailbox -
/// possibly one that does NOT permit custom keywords - can appear at any time,
/// so the AND must be re-derived whenever the folder set might have changed;
/// gating to the initial sync would freeze a now-stale flag and let keyword
/// writeback target a mailbox that rejects it.
///
/// Legacy IMAP derived this every delta cycle too, but for free - it read
/// `supports_custom_keywords` off the per-folder responses it was ALREADY
/// fetching for the CONDSTORE/QRESYNC delta. Bifrost now owns those folder
/// SELECTs inside the engine, so the consumer's aux pass must issue its own
/// SELECT per folder. The remaining steady-state cost (one SELECT per folder
/// per kick) could be cut by a persistent per-folder capability cache that
/// re-probes only newly-appeared folders; that is a stateful optimization,
/// not wired here.
pub async fn run_imap_auxiliary_sync(
    session: &mut imap::connection::ImapSession,
    account_id: &str,
    write_db: &WriteDbState,
    folder_paths: &[String],
) {
    let mut caps = Vec::new();
    for folder in folder_paths {
        match session.select(folder).await {
            Ok(mailbox) => caps.push(imap::client::mailbox_supports_custom_keywords(&mailbox)),
            Err(error) => {
                log::debug!("IMAP keyword-cap SELECT {folder} failed for {account_id}: {error}");
            }
        }
    }
    if caps.is_empty() {
        return;
    }
    let supports_keywords = caps.iter().all(|cap| *cap);
    let aid = account_id.to_string();
    if let Err(error) = write_db
        .with_write(move |conn| {
            db::db::queries_extra::set_account_supports_keywords(conn, &aid, supports_keywords)
        })
        .await
    {
        log::warn!("IMAP keyword-cap write failed for {account_id}: {error}");
    }
}

fn folder_map(folders: &[&ImapFolder]) -> Result<HashMap<String, FolderKind>, String> {
    folders
        .iter()
        .map(|folder| {
            let mapping = map_folder_to_folder(folder)?;
            let kind =
                FolderKind::parse(&mapping.folder_id, common::types::MailProviderKind::Imap)?;
            Ok((folder.path.clone(), kind))
        })
        .collect()
}

fn folder_rows(account_id: &str, folders: &[&ImapFolder]) -> Result<Vec<FolderWriteRow>, String> {
    let path_to_folder_id = folders
        .iter()
        .map(|folder| {
            let mapping = map_folder_to_folder(folder)?;
            Ok::<_, String>((folder.path.as_str(), mapping.folder_id))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    folders
        .iter()
        .map(|folder| {
            let mapping = map_folder_to_folder(folder)?;
            Ok(FolderWriteRow {
                id: mapping.folder_id,
                account_id: account_id.to_string(),
                name: mapping.folder_name,
                visible: None,
                sort_order: None,
                imap_folder_path: Some(folder.raw_path.clone()),
                imap_special_use: folder.special_use.clone(),
                namespace_type: None,
                parent_id: derive_imap_parent_folder_id(
                    &folder.path,
                    &folder.delimiter,
                    &path_to_folder_id,
                ),
                right_read: None,
                right_add: None,
                right_remove: None,
                right_set_seen: None,
                right_set_keywords: None,
                right_create_child: None,
                right_rename: None,
                right_delete: None,
                right_submit: None,
                is_subscribed: None,
                is_undeletable: folder.special_use.is_some(),
            })
        })
        .collect()
}

fn derive_imap_parent_folder_id(
    path: &str,
    delimiter: &str,
    path_to_folder_id: &HashMap<&str, String>,
) -> Option<String> {
    if delimiter.is_empty() {
        return None;
    }
    let last_delim = path.rfind(delimiter)?;
    if last_delim == 0 {
        return None;
    }
    let parent_path = &path[..last_delim];
    path_to_folder_id.get(parent_path).cloned()
}
