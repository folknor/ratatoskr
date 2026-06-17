//! Read-only surface for the main Ratatoskr database.

pub mod blob_hash;
pub mod progress;
pub(crate) mod raw;

pub use raw::{
    ReadCachedStatement, ReadConn, ReadDbState, ReadError, ReadStatement, open_reader_pool,
};

pub mod db {
    pub use crate::{
        ReadCachedStatement, ReadConn, ReadDbState, ReadError, ReadStatement, open_reader_pool,
    };
    // Re-export read-only rusqlite helpers only. Raw Connection,
    // Transaction, and Statement types stay out of the read API.
    pub use rusqlite::types::ToSql;
    pub use rusqlite::{Error as SqlError, OptionalExtension, Row, params};

    pub const DEFAULT_QUERY_LIMIT: i64 = 500;

    pub mod folder_roles;
    pub mod from_row;
    mod from_row_impls;
    pub mod lookups;
    pub mod pinned_searches;
    pub mod queries;
    pub mod queries_extra;
    pub mod sql_fragments;
    pub mod time;
    pub mod types;

    pub use from_row::{FromRow, query_as, query_one};
}
