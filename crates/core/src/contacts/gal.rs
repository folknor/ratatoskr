//! Local GAL cache helpers.
//!
//! Provider directory I/O is owned by the Service resident bifrost account;
//! core retains only the local cache-age query and row type.

use crate::db::ReadDbState;

pub use crate::db::queries_extra::contacts::GalEntry;

pub async fn gal_cache_age(db: &ReadDbState, account_id: String) -> Result<Option<i64>, String> {
    db.with_read(move |conn| {
        crate::db::queries_extra::contacts::gal_cache_age_sync(conn, &account_id)
    })
    .await
}
