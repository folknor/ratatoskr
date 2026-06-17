// Re-export foundation types from db
pub use db_read::db::OptionalExtension;
pub use db_read::db::ReadConn;
pub use db_read::db::ReadDbState;
pub use db_read::db::ReadError;
pub use db_read::db::Row;
pub use db_read::db::SqlError;
pub use db_read::db::ToSql;
pub use db_read::db::from_row;
pub use db_read::db::from_row::{FromRow, query_as, query_one};
pub use db_read::db::open_reader_pool;
pub use db_read::db::params;
pub use db_read::db::pinned_searches;
pub use db_read::db::sql_fragments;
pub use db_read::db::types;
pub use db_read::impl_from_row;

// Core-specific DB modules (queries.rs stays in core due to body_store/crypto deps)
pub mod queries;
pub mod queries_extra;
pub use queries::*;
