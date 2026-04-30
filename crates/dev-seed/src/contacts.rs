use rand::RngExt;
use rusqlite::Connection;

use crate::people::Person;

/// Upsert a sender into contacts and seen_addresses.
pub fn upsert_contact(
    conn: &Connection,
    rng: &mut impl RngExt,
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

/// Seed a handful of locally-created contact groups with realistic names,
/// member counts, and created/updated timestamps. Members are sampled from
/// the people pool by email; threads will already have upserted most of them
/// into `contacts` so the editor's matching-contacts list lights up.
pub fn seed_groups(
    conn: &Connection,
    rng: &mut impl RngExt,
    people: &[Person],
    _accounts: &[crate::accounts::Account],
) -> Result<(), String> {
    if people.is_empty() {
        return Ok(());
    }

    const GROUP_NAMES: &[&str] = &[
        "Engineering",
        "Design",
        "Product",
        "Marketing",
        "Project Phoenix",
        "Project Atlas",
        "Investors",
        "Vendors",
        "Friends",
        "Book Club",
        "Hiking Buddies",
        "Family",
    ];

    let now = chrono::Utc::now().timestamp();
    let one_day = 86_400_i64;

    for name in GROUP_NAMES {
        let group_id = crate::next_uuid(rng);

        // Created 30-365 days ago; updated between then and now.
        let created_days_ago = rng.random_range(30..=365);
        let created_at = now - created_days_ago * one_day;
        let updated_at = if created_days_ago > 1 {
            created_at + rng.random_range(0..created_days_ago) * one_day
        } else {
            created_at
        };

        conn.execute(
            "INSERT INTO contact_groups (id, name, created_at, updated_at, source)
             VALUES (?1, ?2, ?3, ?4, 'user')",
            rusqlite::params![group_id, name, created_at, updated_at],
        )
        .map_err(|e| format!("insert group {name}: {e}"))?;

        // Pick 3-15 unique members from the people pool.
        let member_count = rng.random_range(3..=15.min(people.len()));
        let mut indices: Vec<usize> = (0..people.len()).collect();
        for i in 0..member_count {
            let j = rng.random_range(i..indices.len());
            indices.swap(i, j);
        }

        for &idx in &indices[..member_count] {
            let email = &people[idx].email;
            conn.execute(
                "INSERT OR IGNORE INTO contact_group_members
                 (group_id, member_type, member_value)
                 VALUES (?1, 'email', ?2)",
                rusqlite::params![group_id, email],
            )
            .map_err(|e| format!("insert group member: {e}"))?;
        }
    }

    Ok(())
}

/// Insert VIP senders from the people pool.
pub fn seed_vips(
    conn: &Connection,
    rng: &mut impl RngExt,
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
