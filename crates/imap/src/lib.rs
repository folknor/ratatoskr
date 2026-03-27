pub mod account_config;
pub mod client;
pub mod connection;
pub mod convert;
pub mod folder_mapper;
pub mod imap_delta;
pub mod imap_initial;
pub mod ops;
pub mod parse;
pub mod public_folders;
pub mod raw;
pub mod sync_pipeline;
pub mod types;

/// Check whether an error message indicates a connection-level failure
/// (as opposed to a protocol-level or per-folder error).
///
/// Shared across initial sync, delta sync, and deletion detection so that
/// reconnect decisions use the same heuristics everywhere.
pub(crate) fn is_connection_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("tcp")
        || lower.contains("tls")
        || lower.contains("dns")
        || lower.contains("network")
        || lower.contains("socket")
        || lower.contains("broken pipe")
        || lower.contains("reset by peer")
        || lower.contains("end of file")
        || lower.contains("eof")
        || lower.contains("econnrefused")
}
