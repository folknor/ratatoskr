/// A raw duplicate pair row from the database.
#[derive(Debug, Clone)]
pub struct DuplicatePairRow {
    pub contact_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub source: String,
    pub seen_name: Option<String>,
    pub seen_account_id: String,
}

/// Find contacts that also exist in seen_addresses (duplicate candidates).
pub fn find_contact_duplicates_sync(
    conn: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<DuplicatePairRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.email, c.display_name, c.source,
                    s.display_name AS seen_name, s.account_id AS seen_account_id
             FROM contacts c
             INNER JOIN seen_addresses s ON LOWER(c.email) = LOWER(s.email)
             WHERE c.source != 'seen'
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![limit], |row| {
        Ok(DuplicatePairRow {
            contact_id: row.get("id")?,
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            source: row.get("source")?,
            seen_name: row.get("seen_name")?,
            seen_account_id: row.get::<_, String>("seen_account_id").unwrap_or_default(),
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Update a contact's display name from a seen duplicate (auto-merge).
/// Only updates if the contact's current display_name is NULL.
pub fn merge_seen_duplicate_sync(
    conn: &rusqlite::Connection,
    contact_id: &str,
    seen_display_name: &str,
) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
             WHERE id = ?2 AND display_name IS NULL",
            rusqlite::params![seen_display_name, contact_id],
        )
        .map_err(|e| format!("merge display name: {e}"))?;
    Ok(changed > 0)
}

/// Merge two contacts by ID within a single transaction.
/// The keep contact's NULL fields are filled from the merge contact.
/// Group memberships are migrated. The merge contact is deleted.
pub fn merge_contact_pair_sync(
    conn: &rusqlite::Connection,
    keep_id: &str,
    merge_id: &str,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin merge tx: {e}"))?;

    // Read merge contact's fields
    // TODO(refactor): introduce a MergeContactRow struct instead of the nested-Option tuple.
    #[allow(clippy::type_complexity)]
    let merge_row: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = tx
        .query_row(
            "SELECT display_name, email2, phone, company, notes, avatar_url \
             FROM contacts WHERE id = ?1",
            rusqlite::params![merge_id],
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
    tx.execute(
        "UPDATE contacts SET
           display_name = COALESCE(display_name, ?1),
           email2 = COALESCE(email2, ?2),
           phone = COALESCE(phone, ?3),
           company = COALESCE(company, ?4),
           notes = COALESCE(notes, ?5),
           avatar_url = COALESCE(avatar_url, ?6),
           updated_at = unixepoch()
         WHERE id = ?7",
        rusqlite::params![name, email2, phone, company, notes, avatar_url, keep_id],
    )
    .map_err(|e| format!("merge into keep contact: {e}"))?;

    // Move group memberships from merge to keep
    let keep_email: Option<String> = tx
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            rusqlite::params![keep_id],
            |row| row.get("email"),
        )
        .ok();

    let merge_email: Option<String> = tx
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            rusqlite::params![merge_id],
            |row| row.get("email"),
        )
        .ok();

    if let (Some(ref keep_email), Some(ref merge_email)) = (keep_email, merge_email) {
        tx.execute(
            "UPDATE OR IGNORE contact_group_members \
             SET member_value = ?1 \
             WHERE member_type = 'email' AND member_value = ?2",
            rusqlite::params![keep_email, merge_email],
        )
        .map_err(|e| format!("migrate group memberships: {e}"))?;

        tx.execute(
            "DELETE FROM contact_group_members \
             WHERE member_type = 'email' AND member_value = ?1 \
             AND group_id IN (
               SELECT group_id FROM contact_group_members
               WHERE member_type = 'email' AND member_value = ?2
             )",
            rusqlite::params![merge_email, keep_email],
        )
        .map_err(|e| format!("clean duplicate memberships: {e}"))?;
    }

    // Delete the merge contact
    tx.execute(
        "DELETE FROM contacts WHERE id = ?1",
        rusqlite::params![merge_id],
    )
    .map_err(|e| format!("delete merged contact: {e}"))?;

    tx.commit().map_err(|e| format!("commit merge tx: {e}"))?;
    Ok(())
}
