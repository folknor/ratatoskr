pub mod account;
pub mod auto_responses;
pub mod bimi;
pub mod constants;
pub use store::body_store;
pub mod caldav;
pub mod carddav;
pub mod chat;
pub mod cloud_attachments;
pub mod command_palette_queries;
pub mod contacts;
pub use cmdk as command_palette;
pub use label_colors::preset_colors;
pub use sync::bundling;
pub mod db;
pub mod discovery;
pub mod generation;
#[allow(clippy::single_component_path_imports)]
pub(crate) use graph;
pub use label_colors;
pub use store::inline_image_store;
pub use sync::filters;
pub mod mdn;
pub mod oauth;
pub use ::db_read::blob_hash;
pub use ::db_read::progress;
pub mod provider;
pub mod scope;
// Phase 2 task 6: `core::send` moved to `service::send`; consumers
// import from `service::send` directly. The Phase 5 prerequisite
// retired the `core::actions` shim that briefly re-exported wire-shaped
// `SendAttachment` / `SendRequest`; those types are now reached through
// `service::actions::{SendAttachment, SendRequest}` (or `service::send`).
pub use search;
pub mod search_pipeline;
pub use seen as seen_addresses;
pub use smart_folder;
pub mod send_identity;
pub use smtp;
pub use sync;
pub use sync::smart_labels;
pub use sync::threading;
// Phase 3 task 8: `sync_dispatch` moved to `crates/service/src/sync_dispatch.rs`.
// Service-side callers import directly from `service::sync_dispatch`. The
// transitional `pub use service::sync_dispatch;` re-export was retired in
// Phase 5's prerequisite (it was the second of two edges keeping the
// `rtsk -> service` cycle alive).
pub mod url_cleaning;

// Re-exports for app-layer convenience - avoids direct common dependency.
pub use common::crypto::load_encryption_key;
