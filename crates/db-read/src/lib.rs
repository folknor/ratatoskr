//! Read-only façade over the writer-side `db` crate.
//!
//! This crate exists so `rtsk` and (transitively) `app` can name the
//! database's *read* surface (`ReadConn`, `ReadStatement`,
//! `ReadDbState`, the read-only query modules) without naming the
//! writer's `Connection`, `WriteConn`, `WriterPool`, or any other
//! mutating handle.
//!
//! Every re-export below is named explicitly. **No glob re-exports**:
//! `pub use writer_db::db::*` would pull mutating rusqlite types and
//! the writer pool through `db-read`'s surface, defeating the brokkr
//! `core-no-writer-db` rule that lets `rtsk` depend on `db-read`
//! without depending on the writer crate. The
//! `db_read_public_surface_does_not_reexport_rusqlite` lockdown test
//! pins this invariant; the
//! `db_read_raw_rusqlite_access_is_quarantined` test pins the
//! file-level discipline inside this crate.
//!
//! The clean `ReadConn` / `ReadStatement` / `ReadDbState`
//! implementations live in `writer_db::db` so there is exactly one
//! type underlying both this crate's `db_read::ReadConn` and the
//! writer-side `db::db::ReadConn`. The previous design had two
//! parallel structs (a clean one in `raw.rs` here, a buggy bridge in
//! `writer_db::db`); the trybuild lockdown verified the clean type
//! while every production caller resolved through the bridge.

pub use writer_db::{blob_hash, impl_from_row, impl_from_row_munch, progress};
pub use writer_db::db::{
    ReadCachedStatement, ReadConn, ReadDbState, ReadError, ReadStatement,
};

pub mod db {
    //! Mirror of `writer_db::db` restricted to the read-safe surface.
    //!
    //! Items deliberately absent: `WriteConn`, `WriteTxn`,
    //! `WriterPool`, `apply_writer_pragmas`, `reconcile_velo_rename`,
    //! `open_writer_pool`. Naming any of those from a `db-read`
    //! consumer must fail to resolve.
    //!
    //! `Connection` IS re-exported as a named item below, but only as
    //! a transitional concession: a handful of rtsk wrappers (cloud
    //! attachments, account orchestration, BIMI cache fetch, send
    //! identity selection) still take `&Connection` because they
    //! delegate to writer-side `*_sync` helpers. Those wrappers are
    //! orphan dead code today and slated for deletion in the
    //! follow-up that closes the broader writer-side rtsk surface
    //! (see the "remaining material gaps" review). Until then,
    //! Connection's named re-export is the minimum surface needed
    //! to keep `rtsk` building without re-opening the glob bypass
    //! the lockdown grep was added to catch.

    pub use writer_db::db::{
        // Constants
        DEFAULT_QUERY_LIMIT,
        // Read-side connection wrappers (single canonical types,
        // defined in writer_db::db; re-exported here so existing
        // `rtsk::db::db::ReadConn` paths keep resolving).
        ReadCachedStatement,
        ReadConn,
        ReadDbState,
        ReadError,
        ReadStatement,
        // Reader pool entry point. The writer pool entry
        // (`open_writer_pool`) is intentionally NOT re-exported.
        open_reader_pool,
        // Reader-safe pragma application. The writer counterpart is
        // intentionally NOT re-exported.
        apply_reader_pragmas,
        // rusqlite passthroughs that read code names by type.
        Connection,
        OptionalExtension,
        Row,
        SqlError,
        ToSql,
        params,
        // FromRow trait + helpers, used by every read query that
        // shapes rows into typed structs.
        FromRow,
        from_row,
        query_as,
        query_one,
        // Utility modules safe for read use.
        action_journal,
        folder_roles,
        lookups,
        migrations,
        pending_ops,
        pinned_searches,
        queries,
        queries_extra,
        sql_fragments,
        types,
    };
}
