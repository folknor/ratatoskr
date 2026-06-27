//! B6a retired the per-provider folder-map preparation helpers
//! (`prepare_jmap_mailboxes` / `prepare_graph_folders` /
//! `prepare_gmail_labels`) that once lived here. The list sync is now the
//! single provider-agnostic `bifrost::containers::sync_containers` pass
//! over `SyncEngine::containers_list`. This module is intentionally empty;
//! it is retained only because a source-scan test (`resident.rs`,
//! `push_state_tables_have_no_writer`) `include_str!`s it.
