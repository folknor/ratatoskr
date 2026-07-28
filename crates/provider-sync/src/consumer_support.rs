//! Temporary facade for the Bifrost change-stream consumer.
//!
//! B3a-infra is additive, so the live provider-sync implementations keep
//! calling their helpers in place. The consumer reaches the same narrow set of
//! helpers through this named facade until the provider cutovers move the
//! helpers to their final owner.

// Raw-RFC822 re-parse for JMAP hydration fidelity (B3a-cut-jmap 4.2). The
// consumer recovers the headers / body / attachment detail the bifrost
// structured `Message` drops by re-parsing the `open_raw_rfc822` octets
// through this single shared path - shared so the production consumer and
// the byte-identical golden test cannot diverge.

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
    // The B6a list sync (`bifrost::containers::sync_containers`) already wrote
    // the JMAP folder rows at attach via `containers_list`; re-fetching
    // mailboxes here would double the per-kick request count and trip the
    // section 6.2 `provider_requests max_delta = 0` gate, so the auxiliary
    // pass starts at shared-account discovery.
    crate::jmap::aux_sync::resolve_shared_account_identities(&ctx).await;
    if initial_sync_completed_before_run {
        crate::jmap::aux_sync::poll_share_notifications(&ctx).await;
    }
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
