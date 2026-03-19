pub mod account_resync;

// Re-export everything from the extracted provider-utils crate so that
// existing `crate::provider::*` paths throughout core continue to work.
pub use ratatoskr_provider_utils::attachment_dedup;
pub use ratatoskr_provider_utils::crypto;
pub use ratatoskr_provider_utils::email_parsing;
pub use ratatoskr_provider_utils::encoding;
pub use ratatoskr_provider_utils::folder_roles;
pub use ratatoskr_provider_utils::headers;
pub use ratatoskr_provider_utils::html_sanitizer;
pub use ratatoskr_provider_utils::http;
pub use ratatoskr_provider_utils::label_flags;
pub use ratatoskr_provider_utils::ops;
pub use ratatoskr_provider_utils::parsed_message;
pub use ratatoskr_provider_utils::signature_images;
pub use ratatoskr_provider_utils::text;
pub use ratatoskr_provider_utils::token;
pub use ratatoskr_provider_utils::tracking_pixels;
pub use ratatoskr_provider_utils::types;
