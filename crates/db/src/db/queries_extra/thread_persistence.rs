use std::collections::HashSet;

use mail_parser::MessageParser;
use rusqlite::{Connection, Transaction};

use crate::db::lookups;

pub struct ThreadAggregate {
    pub subject: Option<String>,
    pub snippet: String,
    pub last_date: i64,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
}

/// One message's address fields as raw, possibly-NULL strings from the
/// `messages` table. Each string holds the unparsed RFC 5322 address list
/// for that header; parsing happens downstream in `upsert_thread_participants`.
struct AddressRow {
    from: Option<String>,
    to: Option<String>,
    cc: Option<String>,
    bcc: Option<String>,
}

fn fetch_thread_address_rows(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<AddressRow>, String> {
    let mut stmt = tx
        .prepare(
            "SELECT from_address, to_addresses, cc_addresses, bcc_addresses \
             FROM messages WHERE account_id = ?1 AND thread_id = ?2",
        )
        .map_err(|e| format!("prepare addr: {e}"))?;
    let rows: Vec<AddressRow> = stmt
        .query_map(rusqlite::params![account_id, thread_id], |row| {
            Ok(AddressRow {
                from: row.get(0)?,
                to: row.get(1)?,
                cc: row.get(2)?,
                bcc: row.get(3)?,
            })
        })
        .map_err(|e| format!("query addr: {e}"))?
        .filter_map(Result::ok)
        .collect();
    Ok(rows)
}

pub fn compute_thread_aggregate(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<ThreadAggregate, String> {
    let message_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get("cnt"),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2 AND m.is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, '') AS snippet, date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get("snippet")?, row.get("date")?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get("subject"),
        )
        .map_err(|e| format!("get subject: {e}"))?;

    Ok(ThreadAggregate {
        subject,
        snippet,
        last_date,
        message_count,
        is_read,
        is_starred,
        has_attachments,
    })
}

pub fn upsert_thread_aggregate(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    aggregate: &ThreadAggregate,
    is_important: Option<bool>,
    shared_mailbox_id: Option<&str>,
) -> Result<(), String> {
    let exists: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|c| c > 0)
        .map_err(|e| format!("check thread exists: {e}"))?;

    if exists {
        match is_important {
            Some(is_important) => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, is_important = ?7, \
                     has_attachments = ?8, \
                     shared_mailbox_id = COALESCE(?11, shared_mailbox_id) \
                     WHERE id = ?9 AND account_id = ?10",
                    rusqlite::params![
                        aggregate.subject,
                        aggregate.snippet,
                        aggregate.last_date,
                        aggregate.message_count,
                        aggregate.is_read,
                        aggregate.is_starred,
                        is_important,
                        aggregate.has_attachments,
                        thread_id,
                        account_id,
                        shared_mailbox_id,
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
            None => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, \
                     has_attachments = ?7, \
                     shared_mailbox_id = COALESCE(?10, shared_mailbox_id) \
                     WHERE id = ?8 AND account_id = ?9",
                    rusqlite::params![
                        aggregate.subject,
                        aggregate.snippet,
                        aggregate.last_date,
                        aggregate.message_count,
                        aggregate.is_read,
                        aggregate.is_starred,
                        aggregate.has_attachments,
                        thread_id,
                        account_id,
                        shared_mailbox_id,
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
        }
    } else {
        tx.execute(
            "INSERT INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments, shared_mailbox_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                thread_id,
                account_id,
                aggregate.subject,
                aggregate.snippet,
                aggregate.last_date,
                aggregate.message_count,
                aggregate.is_read,
                aggregate.is_starred,
                is_important.unwrap_or(false),
                aggregate.has_attachments,
                shared_mailbox_id,
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
    }

    Ok(())
}

pub fn ensure_thread_exists(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    tx.execute(
        "INSERT OR IGNORE INTO threads (id, account_id) VALUES (?1, ?2)",
        rusqlite::params![thread_id, account_id],
    )
    .map_err(|e| format!("ensure thread: {e}"))?;
    Ok(())
}

pub fn upsert_thread_participants(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    from_address: Option<&str>,
    to_addresses: Option<&str>,
    cc_addresses: Option<&str>,
    bcc_addresses: Option<&str>,
) -> Result<(), String> {
    let mut emails = HashSet::new();

    if let Some(from) = from_address {
        let parsed = parse_address_list(from);
        for (_, email) in parsed {
            emails.insert(email.to_lowercase());
        }
    }

    for field in [to_addresses, cc_addresses, bcc_addresses]
        .into_iter()
        .flatten()
    {
        let parsed = parse_address_list(field);
        for (_, email) in parsed {
            emails.insert(email.to_lowercase());
        }
    }

    for email in &emails {
        tx.execute(
            "INSERT OR IGNORE INTO thread_participants (account_id, thread_id, email) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, email],
        )
        .map_err(|e| format!("insert thread_participant: {e}"))?;
    }

    Ok(())
}

pub fn query_user_emails(tx: &Transaction) -> Result<Vec<String>, String> {
    let mut stmt = tx
        .prepare("SELECT email FROM accounts")
        .map_err(|e| format!("prepare user emails: {e}"))?;
    let emails: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query user emails: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.to_lowercase())
        .collect();
    Ok(emails)
}

pub fn maybe_update_chat_state(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let chat_email: Option<String> = tx
        .query_row(
            "SELECT cc.email FROM chat_contacts cc \
             INNER JOIN thread_participants tp ON tp.email = cc.email \
             WHERE tp.account_id = ?1 AND tp.thread_id = ?2 \
             LIMIT 1",
            rusqlite::params![account_id, thread_id],
            |row| row.get(0),
        )
        .ok();

    // chat_contacts is keyed by email globally - it persists across all
    // threads with that contact, and is only torn down via the explicit
    // undesignate flow. The early-return paths here just clear the
    // per-thread flag.

    let Some(ref chat_email) = chat_email else {
        tx.execute(
            "UPDATE threads SET is_chat_thread = 0 \
             WHERE account_id = ?1 AND id = ?2 AND is_chat_thread = 1",
            rusqlite::params![account_id, thread_id],
        )
        .map_err(|e| format!("clear chat flag: {e}"))?;
        return Ok(());
    };

    if user_emails.iter().any(|ue| ue == chat_email) {
        tx.execute(
            "UPDATE threads SET is_chat_thread = 0 \
             WHERE account_id = ?1 AND id = ?2 AND is_chat_thread = 1",
            rusqlite::params![account_id, thread_id],
        )
        .map_err(|e| format!("clear chat flag (self-contact): {e}"))?;
        return Ok(());
    }

    let participant_count: i64 = tx
        .query_row(
            "SELECT COUNT(DISTINCT email) FROM thread_participants \
             WHERE account_id = ?1 AND thread_id = ?2",
            rusqlite::params![account_id, thread_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count participants: {e}"))?;

    let has_user = if user_emails.is_empty() {
        false
    } else {
        let placeholders: Vec<&str> = user_emails.iter().map(|_| "?").collect();
        let placeholders_csv = placeholders.join(", ");
        let sql = format!(
            "SELECT COUNT(*) FROM thread_participants \
             WHERE account_id = ?1 AND thread_id = ?2 \
               AND email IN ({placeholders_csv})"
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            Vec::with_capacity(2 + user_emails.len());
        params.push(Box::new(account_id.to_string()));
        params.push(Box::new(thread_id.to_string()));
        for ue in user_emails {
            params.push(Box::new(ue.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();
        tx.query_row(&sql, param_refs.as_slice(), |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            > 0
    };

    let is_chat = participant_count == 2 && has_user;

    tx.execute(
        "UPDATE threads SET is_chat_thread = ?3 \
         WHERE account_id = ?1 AND id = ?2 AND is_chat_thread != ?3",
        rusqlite::params![account_id, thread_id, i32::from(is_chat)],
    )
    .map_err(|e| format!("update chat flag: {e}"))?;

    {
        let thread_latest: Option<(Option<String>, i64)> = tx
            .query_row(
                "SELECT snippet, date FROM messages \
                 WHERE account_id = ?1 AND thread_id = ?2 \
                 ORDER BY date DESC LIMIT 1",
                rusqlite::params![account_id, thread_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        if let Some((preview, ts)) = thread_latest {
            let existing_ts: i64 = tx
                .query_row(
                    "SELECT COALESCE(latest_message_at, 0) FROM chat_contacts WHERE email = ?1",
                    rusqlite::params![chat_email],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            if ts >= existing_ts {
                let unread: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM messages m \
                         INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
                         WHERE t.is_chat_thread = 1 AND m.is_read = 0 \
                           AND LOWER(m.from_address) = ?1",
                        rusqlite::params![chat_email],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);

                let _ = tx.execute(
                    "UPDATE chat_contacts SET latest_message_preview = ?2, \
                     latest_message_at = ?3, unread_count = ?4 WHERE email = ?1",
                    rusqlite::params![chat_email, preview, ts, unread],
                );
            }
        }
    }

    Ok(())
}

pub fn delete_messages_and_cleanup_threads(
    tx: &Transaction,
    account_id: &str,
    message_ids: &[impl AsRef<str>],
) -> Result<Vec<String>, String> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }

    log::debug!(
        "Deleting {} messages and cleaning up threads for account {}",
        message_ids.len(),
        account_id
    );

    let user_emails = query_user_emails(tx)?;

    let mut affected_threads: HashSet<String> = HashSet::new();
    for id in message_ids {
        if let Ok(Some(tid)) = lookups::get_thread_id_for_message(tx, account_id, id.as_ref()) {
            affected_threads.insert(tid);
        }
    }

    for id in message_ids {
        tx.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
            rusqlite::params![account_id, id.as_ref()],
        )
        .map_err(|e| format!("delete message: {e}"))?;
    }

    for tid in &affected_threads {
        let remaining: i64 = tx
            .query_row(
                "SELECT COUNT(*) AS cnt FROM messages WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining: {e}"))?;

        if remaining == 0 {
            tx.execute(
                "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread: {e}"))?;
            tx.execute(
                "DELETE FROM thread_folders WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread folders: {e}"))?;
            tx.execute(
                "DELETE FROM thread_labels WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread labels: {e}"))?;
            tx.execute(
                "DELETE FROM thread_participants WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread participants: {e}"))?;
            // chat_contacts is keyed by email and survives thread deletion -
            // it's only removed via undesignate_chat_contact_sync.
        } else {
            let aggregate = compute_thread_aggregate(tx, account_id, tid)?;
            upsert_thread_aggregate(tx, account_id, tid, &aggregate, None, None)?;

            tx.execute(
                "DELETE FROM thread_participants WHERE account_id = ?1 AND thread_id = ?2",
                rusqlite::params![account_id, tid],
            )
            .map_err(|e| format!("clear thread participants: {e}"))?;
            for row in fetch_thread_address_rows(tx, account_id, tid)? {
                upsert_thread_participants(
                    tx,
                    account_id,
                    tid,
                    row.from.as_deref(),
                    row.to.as_deref(),
                    row.cc.as_deref(),
                    row.bcc.as_deref(),
                )?;
            }
            maybe_update_chat_state(tx, account_id, tid, &user_emails)?;
        }
    }

    Ok(affected_threads.into_iter().collect())
}

pub fn replace_thread_folders<'a>(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    folders: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let unique_folders = filtered_membership_ids(folders);

    tx.execute(
        "DELETE FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete thread folders: {e}"))?;

    for folder_id in unique_folders {
        tx.execute(
            "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, folder_id],
        )
        .map_err(|e| format!("insert thread folder: {e}"))?;
    }

    Ok(())
}

pub fn replace_thread_labels<'a>(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    labels: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let unique_labels = filtered_membership_ids(labels);

    tx.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete thread labels: {e}"))?;

    for label_id in unique_labels {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert thread label: {e}"))?;
    }

    Ok(())
}

/// Add folders observed from a partial provider update without deleting the
/// thread's existing aggregate.
pub fn merge_thread_folders<'a>(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    folders: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    for folder_id in filtered_membership_ids(folders) {
        tx.execute(
            "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, folder_id],
        )
        .map_err(|e| format!("merge thread folder: {e}"))?;
    }

    Ok(())
}

/// Add labels observed from a partial provider update without deleting the
/// thread's existing aggregate. Providers that persist message deltas use this
/// because the DB does not store per-message label membership.
pub fn merge_thread_labels<'a>(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    labels: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    for label_id in filtered_membership_ids(labels) {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("merge thread label: {e}"))?;
    }

    Ok(())
}

fn filtered_membership_ids<'a>(
    labels: impl IntoIterator<Item = &'a str>,
) -> HashSet<&'a str> {
    use crate::db::folder_roles::{is_message_state_label_id, is_reserved_imap_system_keyword};

    labels
        .into_iter()
        .filter(|label_id| !is_message_state_label_id(label_id))
        .filter(|label_id| !is_reserved_imap_system_keyword(label_id))
        .collect()
}

pub fn reassign_messages_and_repair_threads(
    tx: &Transaction,
    account_id: &str,
    new_thread_id: &str,
    message_ids: &[&str],
    user_emails: &[String],
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }

    let old_thread_ids = query_old_thread_ids_for_messages(tx, account_id, new_thread_id, message_ids)?;
    update_message_thread_ids(tx, account_id, new_thread_id, message_ids)?;

    for old_tid in &old_thread_ids {
        repair_thread_after_message_reassignment(tx, account_id, old_tid, user_emails)?;
    }

    rebuild_thread_participants(tx, account_id, new_thread_id)?;
    maybe_update_chat_state(tx, account_id, new_thread_id, user_emails)?;
    Ok(())
}

fn query_old_thread_ids_for_messages(
    tx: &Transaction,
    account_id: &str,
    new_thread_id: &str,
    message_ids: &[&str],
) -> Result<HashSet<String>, String> {
    let mut old_thread_ids = HashSet::new();

    for chunk in message_ids.chunks(100) {
        let placeholders = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "SELECT DISTINCT thread_id FROM messages \
             WHERE account_id = ?1 AND id IN ({placeholders})"
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(1 + chunk.len());
        params.push(Box::new(account_id.to_string()));
        for id in chunk {
            params.push(Box::new((*id).to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();

        let mut stmt = tx
            .prepare(&sql)
            .map_err(|e| format!("prepare old thread query: {e}"))?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
            .map_err(|e| format!("query old thread ids: {e}"))?;
        for tid in rows.flatten() {
            if tid != new_thread_id {
                old_thread_ids.insert(tid);
            }
        }
    }

    Ok(old_thread_ids)
}

fn update_message_thread_ids(
    tx: &Transaction,
    account_id: &str,
    new_thread_id: &str,
    message_ids: &[&str],
) -> Result<(), String> {
    for chunk in message_ids.chunks(100) {
        let placeholders = chunk
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            "UPDATE messages SET thread_id = ?1 \
             WHERE account_id = ?2 AND id IN ({placeholders})"
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(2 + chunk.len());
        params.push(Box::new(new_thread_id.to_string()));
        params.push(Box::new(account_id.to_string()));
        for id in chunk {
            params.push(Box::new((*id).to_string()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| &**p).collect();

        tx.execute(&sql, param_refs.as_slice())
            .map_err(|e| format!("update message thread_ids: {e}"))?;
    }

    Ok(())
}

fn repair_thread_after_message_reassignment(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let remaining: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count remaining in old thread: {e}"))?;

    if remaining == 0 {
        tx.execute(
            "DELETE FROM thread_participants WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
        )
        .map_err(|e| format!("delete orphan thread participants: {e}"))?;
        tx.execute(
            "DELETE FROM thread_labels WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
        )
        .map_err(|e| format!("delete orphan thread labels: {e}"))?;
        tx.execute(
            "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
        )
        .map_err(|e| format!("delete orphan thread: {e}"))?;
        return Ok(());
    }

    rebuild_thread_participants(tx, account_id, thread_id)?;
    maybe_update_chat_state(tx, account_id, thread_id, user_emails)?;
    Ok(())
}

/// One-shot backfill that rebuilds `thread_participants` for every thread
/// in an account that doesn't already have participant rows.
///
/// Pre-launch users who synced before this code was deployed will have
/// `messages` rows but no `thread_participants` data. Without that data,
/// chat designation cannot resolve which threads are 1:1 with the
/// contact, so `is_chat_thread` never flips and the timeline stays
/// empty. The backfill walks each thread missing any participant row,
/// parses the address fields off existing messages, populates
/// `thread_participants`, and then re-evaluates `is_chat_thread` per
/// thread (a no-op when the account has no chat contacts yet, but
/// correct for users who had designated contacts before the participants
/// table was added).
///
/// Idempotent: only threads with no participant rows are processed.
/// Threads that steady-state sync has already populated (per-thread
/// rebuild is the canonical write path) are skipped. The previous
/// per-account "any participant row exists" early-out was too coarse:
/// a single sync-touched thread would suppress backfill of every other
/// stale thread on the account. Returns the number of threads
/// processed.
pub fn backfill_thread_participants_for_account_sync(
    conn: &Connection,
    account_id: &str,
    user_emails: &[String],
) -> Result<usize, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin: {e}"))?;

    let thread_ids: Vec<String> = {
        let mut stmt = tx
            .prepare(
                "SELECT t.id FROM threads t \
                 WHERE t.account_id = ?1 \
                 AND NOT EXISTS ( \
                     SELECT 1 FROM thread_participants p \
                     WHERE p.account_id = t.account_id AND p.thread_id = t.id \
                 )",
            )
            .map_err(|e| format!("prepare threads: {e}"))?;
        stmt.query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("query threads: {e}"))?
            .filter_map(Result::ok)
            .collect()
    };
    if thread_ids.is_empty() {
        // Nothing missing - either the account is fully backfilled or it
        // has no threads. Avoid the empty commit roundtrip.
        return Ok(0);
    }

    let count = thread_ids.len();
    for tid in &thread_ids {
        rebuild_thread_participants(&tx, account_id, tid)?;
        maybe_update_chat_state(&tx, account_id, tid, user_emails)?;
    }

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(count)
}

fn rebuild_thread_participants(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    tx.execute(
        "DELETE FROM thread_participants WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("clear thread participants: {e}"))?;

    for row in fetch_thread_address_rows(tx, account_id, thread_id)? {
        upsert_thread_participants(
            tx,
            account_id,
            thread_id,
            row.from.as_deref(),
            row.to.as_deref(),
            row.cc.as_deref(),
            row.bcc.as_deref(),
        )?;
    }

    Ok(())
}

fn parse_address_list(raw: &str) -> Vec<(Option<String>, String)> {
    if raw.trim().is_empty() {
        return Vec::new();
    }

    let synthetic = format!("To: {raw}\r\n\r\n");
    let parser = MessageParser::default();
    let Some(message) = parser.parse(synthetic.as_bytes()) else {
        return fallback_parse_address_list(raw);
    };

    let Some(to) = message.to() else {
        log::debug!("mail-parser could not extract addresses, using fallback parser");
        return fallback_parse_address_list(raw);
    };

    let mut results = Vec::new();
    for addr in to.iter() {
        if let Some(email) = addr.address.as_ref()
            && email.contains('@')
        {
            let name = addr.name.as_ref().map(ToString::to_string);
            results.push((name, email.to_string()));
        }
    }

    if results.is_empty() {
        return fallback_parse_address_list(raw);
    }

    results
}

fn fallback_parse_address_list(raw: &str) -> Vec<(Option<String>, String)> {
    let mut results = Vec::new();
    for part in raw.split(',') {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(angle_start) = trimmed.rfind('<') {
            if let Some(angle_end) = trimmed[angle_start..].find('>') {
                let email = trimmed[angle_start + 1..angle_start + angle_end].trim();
                if email.contains('@') {
                    let name_part = trimmed[..angle_start].trim().trim_matches('"').trim();
                    let name = if name_part.is_empty() || name_part == email {
                        None
                    } else {
                        Some(name_part.to_string())
                    };
                    results.push((name, email.to_string()));
                }
            }
        } else if trimmed.contains('@') {
            results.push((None, trimmed.to_string()));
        }
    }
    results
}
