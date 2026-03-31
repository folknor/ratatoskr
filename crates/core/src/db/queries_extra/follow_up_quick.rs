use super::super::DbState;
use super::super::types::{DbFollowUpReminder, DbQuickStep, TriggeredFollowUp};
use crate::db::from_row::FromRow;
use rusqlite::params;

pub async fn db_insert_follow_up_reminder(
    db: &DbState,
    id: String,
    account_id: String,
    thread_id: String,
    message_id: String,
    remind_at: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO follow_up_reminders (id, account_id, thread_id, message_id, remind_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
                 ON CONFLICT(account_id, thread_id) DO UPDATE SET
                   message_id = ?4, remind_at = ?5, status = 'pending'",
            params![id, account_id, thread_id, message_id, remind_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_follow_up_for_thread(
    db: &DbState,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbFollowUpReminder>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM follow_up_reminders
                     WHERE account_id = ?1 AND thread_id = ?2 AND status = 'pending' LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![account_id, thread_id], DbFollowUpReminder::from_row)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(reminder)) => Ok(Some(reminder)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

pub async fn db_cancel_follow_up_for_thread(
    db: &DbState,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE follow_up_reminders SET status = 'cancelled'
                 WHERE account_id = ?1 AND thread_id = ?2 AND status = 'pending'",
            params![account_id, thread_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_active_follow_up_thread_ids(
    db: &DbState,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }
    db.with_conn(move |conn| {
        let mut results = Vec::new();
        for chunk in thread_ids.chunks(100) {
            let placeholders = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT thread_id FROM follow_up_reminders
                     WHERE account_id = ?1 AND status = 'pending' AND thread_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for id in chunk {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    row.get::<_, String>("thread_id")
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            results.extend(rows);
        }
        Ok(results)
    })
    .await
}

pub async fn db_check_follow_up_reminders(db: &DbState) -> Result<Vec<TriggeredFollowUp>, String> {
    db.with_conn(move |conn| {
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_secs(),
        )
        .map_err(|_| "current time exceeds i64 range".to_string())?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let reminders: Vec<DbFollowUpReminder> = {
            let mut stmt = tx
                .prepare("SELECT * FROM follow_up_reminders WHERE status = 'pending' AND remind_at <= ?1")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![now], DbFollowUpReminder::from_row)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string());
            drop(stmt);
            rows?
        };

        if reminders.is_empty() {
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(Vec::new());
        }

        let mut triggered = Vec::new();
        for reminder in &reminders {
            let reply_count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) AS cnt FROM messages m
                         WHERE m.account_id = ?1 AND m.thread_id = ?2
                           AND m.date > (SELECT date FROM messages WHERE id = ?3 AND account_id = ?1)
                           AND m.from_address != (SELECT email FROM accounts WHERE id = ?1)",
                    params![reminder.account_id, reminder.thread_id, reminder.message_id],
                    |row| row.get("cnt"),
                )
                .map_err(|e| e.to_string())?;

            if reply_count > 0 {
                tx.execute(
                    "UPDATE follow_up_reminders SET status = 'cancelled' WHERE id = ?1",
                    params![reminder.id],
                )
                .map_err(|e| e.to_string())?;
            } else {
                tx.execute(
                    "UPDATE follow_up_reminders SET status = 'triggered' WHERE id = ?1",
                    params![reminder.id],
                )
                .map_err(|e| e.to_string())?;

                let subject: String = tx
                    .query_row(
                        "SELECT COALESCE(subject, '') AS subject FROM threads WHERE account_id = ?1 AND id = ?2",
                        params![reminder.account_id, reminder.thread_id],
                        |row| row.get("subject"),
                    )
                    .unwrap_or_default();

                triggered.push(TriggeredFollowUp {
                    id: reminder.id.clone(),
                    account_id: reminder.account_id.clone(),
                    thread_id: reminder.thread_id.clone(),
                    subject,
                });
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(triggered)
    })
    .await
}

pub async fn db_get_quick_steps_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM quick_steps WHERE account_id = ?1 ORDER BY sort_order, created_at",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbQuickStep::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_enabled_quick_steps_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbQuickStep>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM quick_steps WHERE account_id = ?1 AND is_enabled = 1
                     ORDER BY sort_order, created_at",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbQuickStep::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_insert_quick_step(db: &DbState, step: DbQuickStep) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO quick_steps (id, account_id, name, description, shortcut, actions_json, icon, is_enabled, continue_on_error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                step.id,
                step.account_id,
                step.name,
                step.description,
                step.shortcut,
                step.actions_json,
                step.icon,
                step.is_enabled,
                step.continue_on_error
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_quick_step(db: &DbState, step: DbQuickStep) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE quick_steps SET name = ?2, description = ?3, shortcut = ?4,
                 actions_json = ?5, icon = ?6, is_enabled = ?7, continue_on_error = ?8
                 WHERE id = ?1",
            params![
                step.id,
                step.name,
                step.description,
                step.shortcut,
                step.actions_json,
                step.icon,
                step.is_enabled,
                step.continue_on_error
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_quick_step(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM quick_steps WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
