//! Contact deduplication and merge logic.
//!
//! When importing or syncing contacts, duplicates are detected by email
//! address. Synced data takes priority over local data for non-overridden
//! fields. The user can also trigger manual merge via the management UI.

use rusqlite::{Connection, params};

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A pair of contacts that share the same email address.
#[derive(Debug, Clone)]
pub struct DuplicatePair {
    /// The email address that is duplicated.
    pub email: String,
    /// The contact ID from the higher-priority source.
    pub primary_id: String,
    /// The contact ID from the lower-priority source.
    pub secondary_id: String,
    /// Display name from primary source.
    pub primary_name: Option<String>,
    /// Display name from secondary source.
    pub secondary_name: Option<String>,
    /// Source of the primary contact.
    pub primary_source: String,
    /// Source of the secondary contact.
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
///
/// Returns pairs where the same email appears in multiple contacts with
/// different IDs. The primary contact is determined by source priority:
/// `user` > `google`/`graph`/`carddav` > `seen`.
pub async fn find_duplicates(db: &DbState) -> Result<Vec<DuplicatePair>, String> {
    db.with_conn(move |conn| find_duplicates_inner(conn))
        .await
}

/// Auto-merge duplicate contacts, preferring synced data over local data
/// for non-overridden fields.
///
/// For each duplicate pair:
/// - Keep the primary contact (higher-priority source)
/// - Merge non-null fields from the secondary into the primary
///   (only if the primary field is null)
/// - Delete the secondary contact
/// - Update any contact group memberships to point to the primary
pub async fn auto_merge_duplicates(db: &DbState) -> Result<MergeResult, String> {
    db.with_conn(move |conn| {
        let duplicates = find_duplicates_inner(conn)?;
        let mut merged_count = 0;
        let mut skipped_count = 0;

        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        for pair in &duplicates {
            match merge_pair(&tx, pair) {
                Ok(true) => merged_count += 1,
                Ok(false) => skipped_count += 1,
                Err(e) => {
                    log::warn!("Failed to merge duplicate {}: {e}", pair.email);
                    skipped_count += 1;
                }
            }
        }

        tx.commit().map_err(|e| format!("commit tx: {e}"))?;

        Ok(MergeResult {
            merged_count,
            skipped_count,
        })
    })
    .await
}

/// Merge two specific contacts by their IDs.
///
/// The `keep_id` contact is preserved; the `merge_id` contact's non-null
/// fields fill in any blanks, then the `merge_id` contact is deleted.
pub async fn merge_contacts(
    db: &DbState,
    keep_id: String,
    merge_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        merge_by_id(&tx, &keep_id, &merge_id)?;

        tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        Ok(())
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

fn find_duplicates_inner(conn: &Connection) -> Result<Vec<DuplicatePair>, String> {
    // Find emails that appear in more than one contact row with different IDs.
    // This shouldn't normally happen since email has a UNIQUE constraint, but
    // it can occur if contacts were inserted with different id prefixes
    // (e.g. "google-acct1-alice@x.com" and "carddav-acct2-alice@x.com" both
    // with email = "alice@x.com"). The UNIQUE constraint on email would prevent
    // this, so duplicates are really about contacts in the contacts table vs
    // the seen_addresses table that share the same email.
    //
    // For the more common case: detect contacts that exist in both `contacts`
    // and `seen_addresses` tables — these can be "promoted" from seen to contact.
    let mut pairs = Vec::new();

    let sql = "SELECT c.id, c.email, c.display_name, c.source,
                      s.display_name AS seen_name, s.account_id AS seen_account_id
               FROM contacts c
               INNER JOIN seen_addresses s ON LOWER(c.email) = LOWER(s.email)
               WHERE c.source != 'seen'
               LIMIT 500";

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(DuplicatePair {
                email: row.get("email")?,
                primary_id: row.get("id")?,
                secondary_id: format!(
                    "seen-{}-{}",
                    row.get::<_, String>("seen_account_id")
                        .unwrap_or_default(),
                    row.get::<_, String>("email").unwrap_or_default()
                ),
                primary_name: row.get("display_name")?,
                secondary_name: row.get("seen_name")?,
                primary_source: row.get("source")?,
                secondary_source: "seen".to_string(),
            })
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        pairs.push(row.map_err(|e| e.to_string())?);
    }

    Ok(pairs)
}

fn merge_pair(conn: &Connection, pair: &DuplicatePair) -> Result<bool, String> {
    let primary_prio = source_priority(&pair.primary_source);
    let secondary_prio = source_priority(&pair.secondary_source);

    // Only merge if sources differ
    if primary_prio == secondary_prio {
        return Ok(false);
    }

    // The pair is contacts vs seen_addresses, so we update the contact's
    // display name if it's null and the seen address has one
    if pair.primary_name.is_none()
        && pair.secondary_name.is_some()
    {
        conn.execute(
            "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
             WHERE id = ?2 AND display_name IS NULL",
            params![pair.secondary_name, pair.primary_id],
        )
        .map_err(|e| format!("merge display name: {e}"))?;
    }

    Ok(true)
}

fn merge_by_id(
    conn: &Connection,
    keep_id: &str,
    merge_id: &str,
) -> Result<(), String> {
    // Get the merge source contact's fields
    let merge_row: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = conn
        .query_row(
            "SELECT display_name, email2, phone, company, notes, avatar_url \
             FROM contacts WHERE id = ?1",
            params![merge_id],
            |row| {
                Ok((
                    row.get("display_name")?,
                    row.get("email2")?,
                    row.get("phone")?,
                    row.get("company")?,
                    row.get("notes")?,
                    row.get("avatar_url")?,
                ))
            },
        )
        .ok();

    let Some((name, email2, phone, company, notes, avatar_url)) = merge_row else {
        return Err(format!("merge contact {merge_id} not found"));
    };

    // Fill in null fields on the keep contact
    conn.execute(
        "UPDATE contacts SET
           display_name = COALESCE(display_name, ?1),
           email2 = COALESCE(email2, ?2),
           phone = COALESCE(phone, ?3),
           company = COALESCE(company, ?4),
           notes = COALESCE(notes, ?5),
           avatar_url = COALESCE(avatar_url, ?6),
           updated_at = unixepoch()
         WHERE id = ?7",
        params![name, email2, phone, company, notes, avatar_url, keep_id],
    )
    .map_err(|e| format!("merge into keep contact: {e}"))?;

    // Move group memberships from merge to keep
    let keep_email: Option<String> = conn
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            params![keep_id],
            |row| row.get("email"),
        )
        .ok();

    let merge_email: Option<String> = conn
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            params![merge_id],
            |row| row.get("email"),
        )
        .ok();

    if let (Some(ref keep_email), Some(ref merge_email)) = (keep_email, merge_email) {
        // Update group memberships: replace merge email with keep email
        conn.execute(
            "UPDATE OR IGNORE contact_group_members \
             SET member_value = ?1 \
             WHERE member_type = 'email' AND member_value = ?2",
            params![keep_email, merge_email],
        )
        .map_err(|e| format!("migrate group memberships: {e}"))?;

        // Delete any remaining duplicate memberships
        conn.execute(
            "DELETE FROM contact_group_members \
             WHERE member_type = 'email' AND member_value = ?1 \
             AND group_id IN (
               SELECT group_id FROM contact_group_members
               WHERE member_type = 'email' AND member_value = ?2
             )",
            params![merge_email, keep_email],
        )
        .map_err(|e| format!("clean duplicate memberships: {e}"))?;
    }

    // Delete the merge contact
    conn.execute("DELETE FROM contacts WHERE id = ?1", params![merge_id])
        .map_err(|e| format!("delete merged contact: {e}"))?;

    Ok(())
}
