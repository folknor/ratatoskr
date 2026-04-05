//! Contact deduplication and merge logic.
//!
//! SQL lives in `db::queries_extra::contacts`. This module keeps
//! source-priority decisions and orchestration.

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Domain types (stay in core)
// ---------------------------------------------------------------------------

/// A pair of contacts that share the same email address.
#[derive(Debug, Clone)]
pub struct DuplicatePair {
    pub email: String,
    pub primary_id: String,
    pub secondary_id: String,
    pub primary_name: Option<String>,
    pub secondary_name: Option<String>,
    pub primary_source: String,
    pub secondary_source: String,
}

/// Result of a merge operation.
#[derive(Debug)]
pub struct MergeResult {
    pub merged_count: usize,
    pub skipped_count: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Find duplicate contacts by email address.
pub async fn find_duplicates(db: &DbState) -> Result<Vec<DuplicatePair>, String> {
    db.with_conn(move |conn| {
        let rows = crate::db::queries_extra::contacts::find_contact_duplicates_sync(
            conn,
            crate::constants::DEFAULT_QUERY_LIMIT,
        )?;
        Ok(rows
            .into_iter()
            .map(|row| DuplicatePair {
                email: row.email.clone(),
                primary_id: row.contact_id,
                secondary_id: format!(
                    "seen-{}-{}",
                    row.seen_account_id, row.email,
                ),
                primary_name: row.display_name,
                secondary_name: row.seen_name,
                primary_source: row.source,
                secondary_source: "seen".to_string(),
            })
            .collect())
    })
    .await
}

/// Auto-merge duplicate contacts using per-pair transactional operations.
///
/// Each pair is merged independently. Partial success is preserved:
/// if one pair fails, earlier successes are not rolled back.
pub async fn auto_merge_duplicates(db: &DbState) -> Result<MergeResult, String> {
    let duplicates = find_duplicates(db).await?;
    let mut merged_count = 0;
    let mut skipped_count = 0;

    for pair in &duplicates {
        match auto_merge_one(db, pair).await {
            Ok(true) => merged_count += 1,
            Ok(false) => skipped_count += 1,
            Err(e) => {
                log::warn!("Failed to merge duplicate {}: {e}", pair.email);
                skipped_count += 1;
            }
        }
    }

    Ok(MergeResult {
        merged_count,
        skipped_count,
    })
}

/// Merge two specific contacts by their IDs.
pub async fn merge_contacts(
    db: &DbState,
    keep_id: String,
    merge_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::merge_contact_pair_sync(conn, &keep_id, &merge_id)
    })
    .await
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Source priority for merge decisions: lower number = higher priority.
fn source_priority(source: &str) -> u8 {
    match source {
        "user" => 0,
        "google" | "graph" | "carddav" | "jmap" => 1,
        _ => 2,
    }
}

async fn auto_merge_one(db: &DbState, pair: &DuplicatePair) -> Result<bool, String> {
    let primary_prio = source_priority(&pair.primary_source);
    let secondary_prio = source_priority(&pair.secondary_source);

    if primary_prio == secondary_prio {
        return Ok(false);
    }

    // For contacts vs seen_addresses: update display name if primary is null
    if pair.primary_name.is_none() {
        if let Some(ref seen_name) = pair.secondary_name {
            let pid = pair.primary_id.clone();
            let name = seen_name.clone();
            db.with_conn(move |conn| {
                crate::db::queries_extra::contacts::merge_seen_duplicate_sync(
                    conn, &pid, &name,
                )
            })
            .await?;
        }
    }

    Ok(true)
}
