use service_state::WriteDbState;

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
/// SELECT per folder. The `folder_paths` are sourced by the caller from the
/// already-synced `folders` table (B6a, spec 4.3), not a re-LIST.
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
