use super::super::DbState;
use super::super::types::{DbTask, DbTaskTag};
use super::dynamic_update;
use rusqlite::{Row, params};

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<DbTask> {
    Ok(DbTask {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        title: row.get("title")?,
        description: row.get("description")?,
        priority: row.get("priority")?,
        is_completed: row.get("is_completed")?,
        completed_at: row.get("completed_at")?,
        due_date: row.get("due_date")?,
        parent_id: row.get("parent_id")?,
        thread_id: row.get("thread_id")?,
        thread_account_id: row.get("thread_account_id")?,
        sort_order: row.get("sort_order")?,
        recurrence_rule: row.get("recurrence_rule")?,
        next_recurrence_at: row.get("next_recurrence_at")?,
        tags_json: row.get("tags_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

fn row_to_task_tag(row: &Row<'_>) -> rusqlite::Result<DbTaskTag> {
    Ok(DbTaskTag {
        tag: row.get("tag")?,
        account_id: row.get("account_id")?,
        color: row.get("color")?,
        sort_order: row.get("sort_order")?,
        created_at: row.get("created_at")?,
    })
}

pub async fn db_get_tasks_for_account(
    db: &DbState,
    account_id: Option<String>,
    include_completed: Option<bool>,
) -> Result<Vec<DbTask>, String> {
    db.with_conn(move |conn| {
        if include_completed.unwrap_or(false) {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND parent_id IS NULL
                         ORDER BY is_completed ASC, sort_order ASC, created_at DESC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_task)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND parent_id IS NULL AND is_completed = 0
                         ORDER BY sort_order ASC, created_at DESC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_task)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
    })
    .await
}

pub async fn db_get_task_by_id(db: &DbState, id: String) -> Result<Option<DbTask>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM tasks WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![id], row_to_task)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(task)) => Ok(Some(task)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

pub async fn db_get_tasks_for_thread(
    db: &DbState,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbTask>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM tasks WHERE thread_account_id = ?1 AND thread_id = ?2
                     ORDER BY is_completed ASC, sort_order ASC, created_at DESC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, thread_id], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_subtasks(db: &DbState, parent_id: String) -> Result<Vec<DbTask>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM tasks WHERE parent_id = ?1 ORDER BY sort_order ASC, created_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![parent_id], row_to_task)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn db_insert_task(
    db: &DbState,
    id: String,
    account_id: Option<String>,
    title: String,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    parent_id: Option<String>,
    thread_id: Option<String>,
    thread_account_id: Option<String>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    tags_json: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO tasks (id, account_id, title, description, priority, due_date, parent_id, thread_id, thread_account_id, sort_order, recurrence_rule, tags_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                id,
                account_id,
                title,
                description,
                priority.unwrap_or_else(|| "none".to_owned()),
                due_date,
                parent_id,
                thread_id,
                thread_account_id,
                sort_order.unwrap_or(0),
                recurrence_rule,
                tags_json.unwrap_or_else(|| "[]".to_owned()),
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn db_update_task(
    db: &DbState,
    id: String,
    title: Option<String>,
    description: Option<String>,
    priority: Option<String>,
    due_date: Option<i64>,
    sort_order: Option<i64>,
    recurrence_rule: Option<String>,
    next_recurrence_at: Option<i64>,
    tags_json: Option<String>,
    clear_description: Option<bool>,
    clear_due_date: Option<bool>,
    clear_recurrence_rule: Option<bool>,
    clear_next_recurrence_at: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        sets.push(("updated_at", Box::new(chrono::Utc::now().timestamp())));
        if let Some(v) = title {
            sets.push(("title", Box::new(v)));
        }
        if clear_description.unwrap_or(false) {
            sets.push(("description", Box::new(Option::<String>::None)));
        } else if let Some(v) = description {
            sets.push(("description", Box::new(Some(v))));
        }
        if let Some(v) = priority {
            sets.push(("priority", Box::new(v)));
        }
        if clear_due_date.unwrap_or(false) {
            sets.push(("due_date", Box::new(Option::<i64>::None)));
        } else if let Some(v) = due_date {
            sets.push(("due_date", Box::new(Some(v))));
        }
        if let Some(v) = sort_order {
            sets.push(("sort_order", Box::new(v)));
        }
        if clear_recurrence_rule.unwrap_or(false) {
            sets.push(("recurrence_rule", Box::new(Option::<String>::None)));
        } else if let Some(v) = recurrence_rule {
            sets.push(("recurrence_rule", Box::new(Some(v))));
        }
        if clear_next_recurrence_at.unwrap_or(false) {
            sets.push(("next_recurrence_at", Box::new(Option::<i64>::None)));
        } else if let Some(v) = next_recurrence_at {
            sets.push(("next_recurrence_at", Box::new(Some(v))));
        }
        if let Some(v) = tags_json {
            sets.push(("tags_json", Box::new(v)));
        }

        dynamic_update(conn, "tasks", "id", &id, sets)
    })
    .await
}

pub async fn db_delete_task(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_complete_task(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE tasks SET is_completed = 1, completed_at = ?2, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_uncomplete_task(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE tasks SET is_completed = 0, completed_at = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_reorder_tasks(db: &DbState, task_ids: Vec<String>) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp();
        for (i, task_id) in task_ids.iter().enumerate() {
            tx.execute(
                "UPDATE tasks SET sort_order = ?1, updated_at = ?3 WHERE id = ?2",
                params![i as i64, task_id, now],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_incomplete_task_count(
    db: &DbState,
    account_id: Option<String>,
) -> Result<i64, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE (account_id = ?1 OR account_id IS NULL) AND is_completed = 0",
            params![account_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_task_tags(
    db: &DbState,
    account_id: Option<String>,
) -> Result<Vec<DbTaskTag>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM task_tags WHERE account_id = ?1 OR account_id IS NULL ORDER BY sort_order ASC")
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], row_to_task_tag)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_upsert_task_tag(
    db: &DbState,
    tag: String,
    account_id: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO task_tags (tag, account_id, color)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(tag, account_id) DO UPDATE SET color = ?3",
            params![tag, account_id, color],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_task_tag(
    db: &DbState,
    tag: String,
    account_id: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM task_tags WHERE tag = ?1 AND account_id = ?2",
            params![tag, account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
