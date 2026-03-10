// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::params;
use tauri::State;

use crate::db::DbState;
use super::{insert_label, remove_inbox_label, remove_label};

// ── Archive ──────────────────────────────────────────────────

#[tauri::command]
pub async fn email_action_archive(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            remove_inbox_label(conn, &account_id, &thread_id)
        })
        .await
}

// ── Trash ────────────────────────────────────────────────────

#[tauri::command]
pub async fn email_action_trash(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            remove_inbox_label(&tx, &account_id, &thread_id)?;
            insert_label(&tx, &account_id, &thread_id, "TRASH")?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Permanent delete ─────────────────────────────────────────

#[tauri::command]
pub async fn email_action_permanent_delete(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM threads WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Spam / Not spam ──────────────────────────────────────────

#[tauri::command]
pub async fn email_action_spam(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_spam: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            if is_spam {
                remove_inbox_label(&tx, &account_id, &thread_id)?;
                insert_label(&tx, &account_id, &thread_id, "SPAM")?;
            } else {
                remove_label(&tx, &account_id, &thread_id, "SPAM")?;
                insert_label(&tx, &account_id, &thread_id, "INBOX")?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Mark read / unread ───────────────────────────────────────

#[tauri::command]
pub async fn email_action_mark_read(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_read: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_read = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_read],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Star / unstar ────────────────────────────────────────────

#[tauri::command]
pub async fn email_action_star(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_starred: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE threads SET is_starred = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, is_starred],
            )
            .map_err(|e| e.to_string())?;
            if is_starred {
                insert_label(&tx, &account_id, &thread_id, "STARRED")?;
            } else {
                remove_label(&tx, &account_id, &thread_id, "STARRED")?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Snooze ───────────────────────────────────────────────────

#[tauri::command]
pub async fn email_action_snooze(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    snooze_until: i64,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE threads SET is_snoozed = 1, snooze_until = ?3 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id, snooze_until],
            )
            .map_err(|e| e.to_string())?;
            remove_inbox_label(&tx, &account_id, &thread_id)?;
            insert_label(&tx, &account_id, &thread_id, "SNOOZED")?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Unsnooze ─────────────────────────────────────────────────

#[tauri::command]
pub async fn email_action_unsnooze(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE threads SET is_snoozed = 0, snooze_until = NULL WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            remove_label(&tx, &account_id, &thread_id, "SNOOZED")?;
            insert_label(&tx, &account_id, &thread_id, "INBOX")?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Pin / unpin (local only) ─────────────────────────────────

#[tauri::command]
pub async fn email_action_pin(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_pinned = 1 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

#[tauri::command]
pub async fn email_action_unpin(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_pinned = 0 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Mute (archives + sets flag) ──────────────────────────────

#[tauri::command]
pub async fn email_action_mute(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute(
                "UPDATE threads SET is_muted = 1 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            remove_inbox_label(&tx, &account_id, &thread_id)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Unmute (local only) ──────────────────────────────────────

#[tauri::command]
pub async fn email_action_unmute(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE threads SET is_muted = 0 WHERE account_id = ?1 AND id = ?2",
                params![account_id, thread_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
}

// ── Add / remove label ──────────────────────────────────────

#[tauri::command]
pub async fn email_action_add_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            insert_label(conn, &account_id, &thread_id, &label_id)
        })
        .await
}

#[tauri::command]
pub async fn email_action_remove_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            remove_label(conn, &account_id, &thread_id, &label_id)
        })
        .await
}

// ── Move to folder ──────────────────────────────────────────

#[tauri::command]
pub async fn email_action_move_to_folder(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    folder_label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            insert_label(conn, &account_id, &thread_id, &folder_label_id)
        })
        .await
}

// ── Centralized pending operation queue ──────────────────────

#[tauri::command]
pub async fn db_enqueue_pending_operation(
    state: State<'_, DbState>,
    id: String,
    account_id: String,
    operation_type: String,
    resource_id: String,
    params_json: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO pending_operations (id, account_id, operation_type, resource_id, params, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
                params![id, account_id, operation_type, resource_id, params_json],
            )
            .map_err(|e| format!("enqueue pending op: {e}"))?;
            Ok(())
        })
        .await
}
