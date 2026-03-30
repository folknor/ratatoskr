use rand::Rng;
use rusqlite::Connection;

use crate::people::Person;

/// Upsert a sender into contacts and seen_addresses.
pub fn upsert_contact(
    conn: &Connection,
    rng: &mut impl Rng,
    email: &str,
    display_name: &str,
    account_id: &str,
    date: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, frequency, last_contacted_at)
         VALUES (?1, ?2, ?3, 1, ?4)
         ON CONFLICT(email) DO UPDATE SET
             frequency = frequency + 1,
             display_name = COALESCE(excluded.display_name, display_name),
             last_contacted_at = MAX(COALESCE(excluded.last_contacted_at, 0),
                                     COALESCE(last_contacted_at, 0))",
        rusqlite::params![crate::next_uuid(rng), email, display_name, date],
    )
    .map_err(|e| format!("upsert contact: {e}"))?;

    conn.execute(
        "INSERT INTO seen_addresses (email, account_id, display_name,
         times_received_from, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(account_id, email) DO UPDATE SET
             times_received_from = times_received_from + 1,
             display_name = COALESCE(excluded.display_name, display_name),
             last_seen_at = MAX(excluded.last_seen_at, last_seen_at)",
        rusqlite::params![email, account_id, display_name, date],
    )
    .map_err(|e| format!("upsert seen_address: {e}"))?;

    Ok(())
}

/// Insert VIP senders from the people pool.
pub fn seed_vips(
    conn: &Connection,
    rng: &mut impl Rng,
    people: &[Person],
    accounts: &[crate::accounts::Account],
) -> Result<(), String> {
    let count = 5.min(people.len());
    // Sample 5 random people as VIPs
    let mut indices: Vec<usize> = (0..people.len()).collect();
    // Fisher-Yates partial shuffle
    for i in 0..count {
        let j = rng.random_range(i..indices.len());
        indices.swap(i, j);
    }

    for &idx in &indices[..count] {
        let person = &people[idx];
        // Add VIP for the first account
        let account_id = &accounts[0].id;
        conn.execute(
            "INSERT OR IGNORE INTO notification_vips (id, account_id, email_address, display_name)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                crate::next_uuid(rng),
                account_id,
                person.email,
                person.display_name,
            ],
        )
        .map_err(|e| format!("insert vip: {e}"))?;
    }

    Ok(())
}
