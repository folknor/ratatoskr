//! Temporary facade for the Bifrost change-stream consumer.
//!
//! B3a-infra is additive, so the live provider-sync implementations keep
//! calling their helpers in place. The consumer reaches the same narrow set of
//! helpers through this named facade until the provider cutovers move the
//! helpers to their final owner.

use std::collections::HashMap;

use common::types::FolderKind;

// Raw-RFC822 re-parse for JMAP hydration fidelity (B3a-cut-jmap 4.2). The
// consumer recovers the headers / body / attachment detail the bifrost
// structured `Message` drops by re-parsing the `open_raw_rfc822` octets
// through this single shared path - shared so the production consumer and
// the byte-identical golden test cannot diverge.
pub use ::jmap::rfc822::{
    Rfc822Attachment, Rfc822Parsed, format_addr_field, parse_rfc822, snippet_from_body,
};

// Keyword membership: B3a-infra's baseline write applies per-message
// keywords (and the thread keyword-label rollup) for every provider.
pub use crate::keyword_membership::{
    KeywordProvider, recompute_thread_keyword_labels, replace_message_keywords,
};
// All three store writes are used by the baseline write (body + inline +
// search), per spec 4.1.5.
pub use crate::persistence::{index_search_documents, store_inline_images, store_message_bodies};
// Folder-row creation (spec 1 / 4.1.4: the baseline membership surface
// includes "folder-row creation"). The message_folders FK targets
// folders(account_id, id), so the consumer must ensure the folder row
// exists before writing membership.
pub use db::db::queries_extra::{FolderWriteRow, insert_folders_batch};
// Reached unchanged by the marker-gated post-persist arm. NOTE: the
// consumer's seen-ingest is re-implemented inline in `post_persist.rs`
// rather than calling this helper, because the marker insert MUST share the
// same `with_write` txn as the counter increment and this helper opens its
// own txn (spec 4.1.3). Kept exported so the cut specs reuse the canonical
// entry point.
pub use crate::seen_ingest::ingest_from_messages;
pub use crate::thread_membership::{
    // cut-spec: JMAP folders-only strategy (reserved; deviates from the
    // spec's "baseline" naming, which the implementation upgrades to the
    // folders+labels recompute so the membership gate can assert label rows).
    replace_message_folders_and_recompute,
    // B3a-infra BASELINE membership write (folders + labels + thread rollup).
    replace_message_membership_and_recompute,
    // cut-spec: Gmail full-coverage thread membership replace (reserved).
    replace_thread_membership_from_full_coverage,
};

#[allow(clippy::too_many_arguments)]
pub async fn run_jmap_auxiliary_sync(
    client: &crate::jmap::client::JmapClient,
    account_id: &str,
    read_db: &db::db::ReadDbState,
    write_db: &service_state::WriteDbState,
    initial_sync_completed_before_run: bool,
) {
    let ctx = crate::jmap::aux_sync::AuxiliarySyncCtx {
        client,
        account_id,
        read_db,
        write_db,
    };

    // NOTE: the mailbox enumeration + folder-row write is NOT re-issued here.
    // The runner already fetched mailboxes once this kick via
    // `sync_jmap_mailbox_folder_map` (which drives the same `sync_mailboxes`
    // and writes the folder rows) to build the consumer's folder map. A
    // second `Mailbox/get` here would double the per-kick request count and
    // trip the section 6.2 `provider_requests max_delta = 0` gate, so the
    // auxiliary pass shares that single fetch and starts at shared-account
    // discovery.
    crate::jmap::aux_sync::discover_shared_accounts(&ctx).await;
    crate::jmap::aux_sync::resolve_shared_account_identities(&ctx).await;
    if initial_sync_completed_before_run {
        crate::jmap::aux_sync::poll_share_notifications(&ctx).await;
        match crate::jmap::contacts_sync::jmap_contacts_delta_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            Ok(count) if count > 0 => {
                log::info!("[JMAP] Contacts delta sync: {count} affected for account {account_id}");
            }
            Ok(_) => {}
            Err(error) => {
                log::warn!("[JMAP] Contacts delta sync failed for account {account_id}: {error}");
            }
        }
    } else {
        match crate::jmap::contacts_sync::jmap_contacts_initial_sync(
            client,
            account_id,
            read_db,
            &write_db.writer_pool(),
        )
        .await
        {
            Ok(count) => {
                log::info!(
                    "[JMAP] Initial contacts sync: {count} contacts for account {account_id}"
                );
            }
            Err(error) => {
                log::warn!("[JMAP] Contacts initial sync failed for account {account_id}: {error}");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn sync_jmap_mailbox_folder_map(
    client: &crate::jmap::client::JmapClient,
    account_id: &str,
    read_db: &db::db::ReadDbState,
    write_db: &service_state::WriteDbState,
) -> Result<HashMap<String, FolderKind>, String> {
    let ctx = crate::jmap::aux_sync::AuxiliarySyncCtx {
        client,
        account_id,
        read_db,
        write_db,
    };
    crate::jmap::aux_sync::sync_mailbox_folder_map(&ctx).await
}

#[allow(clippy::too_many_arguments)]
pub async fn sync_graph_folder_map(
    client: &crate::graph::client::GraphClient,
    account_id: &str,
    read_db: &db::db::ReadDbState,
    _write_db: &service_state::WriteDbState,
) -> Result<HashMap<String, FolderKind>, String> {
    crate::graph::aux_sync::sync_graph_folder_map(client, account_id, read_db).await
}

#[allow(clippy::too_many_arguments)]
pub async fn run_graph_auxiliary_sync(
    client: &crate::graph::client::GraphClient,
    account_id: &str,
    read_db: &db::db::ReadDbState,
    write_db: &service_state::WriteDbState,
    initial_sync_completed_before_run: bool,
) {
    crate::graph::aux_sync::run_graph_auxiliary_sync(
        client,
        account_id,
        read_db,
        write_db,
        initial_sync_completed_before_run,
    )
    .await;
}
