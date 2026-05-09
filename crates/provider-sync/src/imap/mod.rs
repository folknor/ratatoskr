pub use ::imap::{client, connection, convert, folder_mapper, types};

pub mod imap_delta;
pub mod imap_delta_janitor;
pub mod imap_initial;
pub mod sync_pipeline;

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
