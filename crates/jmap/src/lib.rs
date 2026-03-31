pub mod calendar_sync;
pub mod client;
pub mod contacts_sync;

/// Maximum number of changes to request per JMAP `Foo/changes` call.
///
/// Must be > 0 (JMAP spec forbids 0).  500 is a reasonable batch that keeps
/// round-trips low while avoiding excessively large responses.
pub const JMAP_MAX_CHANGES: usize = 500;
pub mod helpers;
pub mod mailbox_mapper;
pub mod ops;
pub mod parse;
pub mod push;
pub mod shared_mailbox_sync;
pub mod sieve;
pub mod signatures;
pub mod sync;
