//! MDN (read-receipt) response dispatch.
//!
//! Triggered from the mark-as-read code path. For each message in the
//! thread that requested a read receipt and hasn't had one sent yet,
//! resolve the per-sender policy and, on `Always`, build an RFC 8098
//! disposition-notification (`rtsk::mdn::build_mdn_message`) and submit
//! the pre-assembled bytes through the resident engine's raw-send
//! passthrough (`engine.send_raw_message`) - bifrost's structured
//! `SendRequest` cannot express a `multipart/report`, so the MDN keeps
//! ratatoskr's assembler while the transport is bifrost's.
//!
//! Failures are soft: a send error logs and continues to the next
//! candidate. Unmarking the read state would be a worse outcome than a
//! missing receipt.

use bifrost_types::{AccountId, ObjectId};
use bytes::Bytes;
use db::db::ReadConn;
use db::db::queries_extra::mdn::{
    ReadReceiptPolicy, mark_mdn_sent_local, resolve_read_receipt_policy,
};
use rtsk::mdn::build_mdn_message;
use rusqlite::params;

use super::context::ActionContext;
use super::dispatch_target::{engine_error_to_action_error, resolve_message_object_id};
use crate::bifrost::resident::ResidentActionAccount;

struct Candidate {
    message_id: String,
    from_address: String,
    message_id_header: String,
}

/// Iterate the thread's read-receipt-requested messages and send any
/// MDNs the policy authorises.
pub(crate) async fn send_mdn_responses(
    ctx: &ActionContext,
    action_account: &ResidentActionAccount,
    account_id: &str,
    thread_id: &str,
) {
    let candidates = match collect_candidates(ctx, account_id, thread_id).await {
        Ok(c) if c.is_empty() => return,
        Ok(c) => c,
        Err(e) => {
            log::warn!("[mdn] candidate lookup failed for {account_id}/{thread_id}: {e}");
            return;
        }
    };

    let (account_email, account_display_name) = match load_account_identity(ctx, account_id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => {
            log::warn!("[mdn] account {account_id} missing during MDN dispatch");
            return;
        }
        Err(e) => {
            log::warn!("[mdn] account lookup failed for {account_id}: {e}");
            return;
        }
    };

    for candidate in candidates {
        let policy = resolve_policy(ctx, account_id, &candidate.from_address).await;
        if !matches!(policy, ReadReceiptPolicy::Always) {
            // Ask is treated as Never until the prompt UI ships.
            continue;
        }

        let raw = build_mdn_message(
            &candidate.from_address,
            &candidate.message_id_header,
            &account_email,
            account_display_name.as_deref().unwrap_or(""),
            false,
        );
        match action_account
            .engine
            .send_raw_message(&AccountId(account_id.to_string()), Bytes::from(raw), None)
            .await
            .map_err(engine_error_to_action_error)
        {
            Ok(_) => {
                if let Err(e) = mark_sent(ctx, account_id, &candidate.message_id).await {
                    log::warn!(
                        "[mdn] sent OK but failed to mark mdn_sent for \
                         {account_id}/{}: {e}",
                        candidate.message_id
                    );
                }
                // Server-side keyword sync. Soft-fail: cross-client
                // coordination is best-effort; the local mdn_sent flag
                // already prevents our own double-sends.
                match resolve_message_object_id(
                    ctx,
                    account_id,
                    &candidate.message_id,
                    action_account.provider,
                )
                .await
                {
                    Ok(object_id) => {
                        if let Err(e) = action_account
                            .engine
                            .mark_mdn_sent(
                                &AccountId(account_id.to_string()),
                                ObjectId(object_id.0),
                            )
                            .await
                            .map_err(engine_error_to_action_error)
                        {
                            log::warn!(
                                "[mdn] server keyword sync failed for {account_id}/{}: {e}",
                                candidate.message_id
                            );
                        }
                    }
                    Err(e) => log::warn!(
                        "[mdn] server keyword id resolution failed for {account_id}/{}: {e}",
                        candidate.message_id
                    ),
                }
            }
            Err(e) => {
                log::warn!(
                    "[mdn] send failed for {account_id}/{}: {e}",
                    candidate.message_id
                );
            }
        }
    }
}

/// Engine-dispatch entry point for the read-receipt write-back.
///
/// The action pipeline marks a thread read through the resident bifrost
/// engine (`bulk_set_flags`), which has no MDN hook of its own. On a
/// successful mark-as-read, send any authorised MDNs for the thread.
///
/// Both call sites (the bulk and single-thread mark-as-read success arms
/// in `batch.rs`) already hold a resolved `action_account`, so the handle
/// is passed in directly rather than re-resolved per mark-as-read; the
/// MDN send rides the SAME resident engine (`engine.send_raw_message`) the
/// keep-attached mutation dispatch uses. The candidate query runs first so
/// the common no-receipt thread never pays for the dispatch.
pub(crate) async fn send_mdn_for_read(
    ctx: &ActionContext,
    action_account: &ResidentActionAccount,
    account_id: &str,
    thread_id: &str,
) {
    match collect_candidates(ctx, account_id, thread_id).await {
        Ok(c) if c.is_empty() => return,
        Ok(_) => {}
        Err(e) => {
            log::warn!("[mdn] candidate lookup failed for {account_id}/{thread_id}: {e}");
            return;
        }
    }

    send_mdn_responses(ctx, action_account, account_id, thread_id).await;
}

fn collect_candidates_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<Candidate>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, from_address, message_id_header \
             FROM messages \
             WHERE account_id = ?1 \
               AND thread_id = ?2 \
               AND mdn_requested = 1 \
               AND mdn_sent = 0 \
               AND from_address IS NOT NULL \
               AND message_id_header IS NOT NULL",
        )
        .map_err(|e| format!("prepare candidate query: {e}"))?;
    let rows = stmt
        .query_map(params![account_id, thread_id], |row| {
            Ok(Candidate {
                message_id: row.get::<_, String>(0)?,
                from_address: row.get::<_, String>(1)?,
                message_id_header: row.get::<_, String>(2)?,
            })
        })
        .map_err(|e| format!("query candidates: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect candidates: {e}"))
}

async fn collect_candidates(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<Candidate>, String> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| collect_candidates_sync(&conn.as_read(), &aid, &tid))
        .await
}

async fn load_account_identity(
    ctx: &ActionContext,
    account_id: &str,
) -> Result<Option<(String, Option<String>)>, String> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    db.with_write(move |conn| {
        Ok(conn
            .query_row(
                "SELECT email, display_name FROM accounts WHERE id = ?1",
                params![aid],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .ok())
    })
    .await
}

async fn resolve_policy(ctx: &ActionContext, account_id: &str, sender: &str) -> ReadReceiptPolicy {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let sender = sender.to_string();
    db.with_write(move |conn| Ok(resolve_read_receipt_policy(conn, &aid, &sender)))
        .await
        .unwrap_or(ReadReceiptPolicy::Never)
}

async fn mark_sent(ctx: &ActionContext, account_id: &str, message_id: &str) -> Result<(), String> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let mid = message_id.to_string();
    db.with_write(move |conn| mark_mdn_sent_local(conn, &aid, &mid))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open db");
        conn.execute_batch(
            "CREATE TABLE messages (
                id TEXT NOT NULL,
                account_id TEXT NOT NULL,
                thread_id TEXT,
                from_address TEXT,
                message_id_header TEXT,
                mdn_requested INTEGER NOT NULL DEFAULT 0,
                mdn_sent INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (account_id, id)
            );",
        )
        .expect("create table");
        conn
    }

    fn insert(
        conn: &Connection,
        id: &str,
        thread_id: &str,
        from: Option<&str>,
        msg_id: Option<&str>,
        mdn_requested: bool,
        mdn_sent: bool,
    ) {
        conn.execute(
            "INSERT INTO messages \
                 (id, account_id, thread_id, from_address, message_id_header, \
                  mdn_requested, mdn_sent) \
             VALUES (?1, 'acct', ?2, ?3, ?4, ?5, ?6)",
            params![id, thread_id, from, msg_id, mdn_requested, mdn_sent],
        )
        .expect("insert");
    }

    #[test]
    fn collects_only_pending_requested_messages() {
        let conn = setup_db();
        // Wanted: requested, not sent, both fields populated.
        insert(
            &conn,
            "m-want",
            "t1",
            Some("a@x"),
            Some("<mid-1>"),
            true,
            false,
        );
        // Excluded: already sent.
        insert(
            &conn,
            "m-sent",
            "t1",
            Some("b@x"),
            Some("<mid-2>"),
            true,
            true,
        );
        // Excluded: not requested.
        insert(
            &conn,
            "m-norq",
            "t1",
            Some("c@x"),
            Some("<mid-3>"),
            false,
            false,
        );
        // Excluded: missing message_id_header.
        insert(&conn, "m-nomid", "t1", Some("d@x"), None, true, false);
        // Excluded: missing from_address.
        insert(&conn, "m-nofrom", "t1", None, Some("<mid-4>"), true, false);
        // Excluded: different thread.
        insert(
            &conn,
            "m-other",
            "t2",
            Some("e@x"),
            Some("<mid-5>"),
            true,
            false,
        );

        let candidates =
            collect_candidates_sync(&ReadConn::from_raw(&conn), "acct", "t1").expect("query");
        let ids: Vec<&str> = candidates.iter().map(|c| c.message_id.as_str()).collect();
        assert_eq!(ids, vec!["m-want"]);
    }

    #[test]
    fn returns_all_pending_in_thread() {
        let conn = setup_db();
        insert(&conn, "m1", "t1", Some("a@x"), Some("<1>"), true, false);
        insert(&conn, "m2", "t1", Some("b@x"), Some("<2>"), true, false);

        let candidates =
            collect_candidates_sync(&ReadConn::from_raw(&conn), "acct", "t1").expect("query");
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn empty_for_thread_with_no_requests() {
        let conn = setup_db();
        insert(&conn, "m1", "t1", Some("a@x"), Some("<1>"), false, false);

        let candidates =
            collect_candidates_sync(&ReadConn::from_raw(&conn), "acct", "t1").expect("query");
        assert!(candidates.is_empty());
    }
}
