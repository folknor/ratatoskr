pub mod account_resync;

// Re-export everything from the extracted common crate so that
// existing `crate::provider::*` paths throughout core continue to work.
pub use common::attachment_dedup;
pub use common::crypto;
pub use common::email_parsing;
pub use common::encoding;
pub use common::error;
pub use common::folder_roles;
pub use common::headers;
pub use common::html_sanitizer;
pub use common::http;
pub use common::label_flags;
pub use common::ops;
pub use common::parsed_message;
pub use common::signature_images;
pub use common::text;
pub use common::token;
pub use common::tracking_pixels;
pub use common::typed_ids;
pub use common::types;
