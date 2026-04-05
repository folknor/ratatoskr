// Re-export foundation types from db
pub use db::db::DbState;
pub use db::db::ReadWriteDb;
pub use db::db::Connection;
pub use db::db::Row;
pub use db::db::SqlError;
pub use db::db::OptionalExtension;
pub use db::db::params;
pub use db::db::from_row;
pub use db::db::from_row::{FromRow, query_as, query_one};
pub use db::db::migrations;
pub use db::db::pinned_searches;
pub use db::db::sql_fragments;
pub use db::db::types;
pub use db::impl_from_row;

// Re-export pending_ops from db crate
pub use db::db::pending_ops;

// Core-specific DB modules (queries.rs stays in core due to body_store/crypto deps)
pub mod queries;
pub mod queries_extra;
pub use queries::*;
