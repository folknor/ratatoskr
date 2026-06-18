//! Temporary facade for the Bifrost change-stream consumer.
//!
//! B3a-infra is additive, so the live provider-sync implementations keep
//! calling their helpers in place. The consumer reaches the same narrow set of
//! helpers through this named facade until the provider cutovers move the
//! helpers to their final owner.

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
