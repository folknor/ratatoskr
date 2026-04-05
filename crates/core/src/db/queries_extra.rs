// Re-export all moved modules from the db crate.
pub use db::db::queries_extra::*;

// navigation.rs remains in core: depends on smart_folder::count_smart_folder_unread,
// and smart-folder depends on db (cycle).
pub mod navigation;
pub use navigation::*;
