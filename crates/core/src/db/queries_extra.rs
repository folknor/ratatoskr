// Re-export all moved modules from the db crate.
pub use db::db::queries_extra::*;

// These three modules remain in core due to circular dependency constraints:
// - navigation.rs depends on SYSTEM_FOLDER_ROLES (common crate) and get_labels (core's queries.rs)
// - thread_detail.rs depends on resolve_label_color (label-colors crate) and SYSTEM_FOLDER_ROLES
// - thread_ui_state.rs is tightly coupled to thread_detail.rs
pub mod navigation;
pub mod thread_detail;
mod thread_ui_state;

pub use navigation::*;
pub use thread_detail::*;
pub use thread_ui_state::*;
