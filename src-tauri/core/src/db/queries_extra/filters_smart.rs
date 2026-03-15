use super::super::DbState;
use super::super::types::{DbFilterRule, DbSmartFolder, DbSmartLabelRule, SortOrderItem};
use super::dynamic_update;
use crate::db::from_row::FromRow;
use rusqlite::params;

pub async fn db_get_filters_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbFilterRule>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM filter_rules WHERE account_id = ?1 ORDER BY sort_order, created_at",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbFilterRule::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_insert_filter(
    db: &DbState,
    id: String,
    account_id: String,
    name: String,
    criteria_json: String,
    actions_json: String,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO filter_rules (id, account_id, name, is_enabled, criteria_json, actions_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                account_id,
                name,
                is_enabled.unwrap_or(true),
                criteria_json,
                actions_json
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_filter(
    db: &DbState,
    id: String,
    name: Option<String>,
    criteria_json: Option<String>,
    actions_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = criteria_json {
            sets.push(("criteria_json", Box::new(v)));
        }
        if let Some(v) = actions_json {
            sets.push(("actions_json", Box::new(v)));
        }
        if let Some(v) = is_enabled {
            sets.push(("is_enabled", Box::new(v)));
        }
        dynamic_update(conn, "filter_rules", "id", &id, sets)
    })
    .await
}

pub async fn db_delete_filter(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM filter_rules WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_smart_folders(
    db: &DbState,
    account_id: Option<String>,
) -> Result<Vec<DbSmartFolder>, String> {
    db.with_conn(move |conn| {
        if let Some(ref aid) = account_id {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM smart_folders WHERE account_id IS NULL OR account_id = ?1
                         ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![aid], DbSmartFolder::from_row)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT * FROM smart_folders WHERE account_id IS NULL
                         ORDER BY sort_order, created_at",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map([], DbSmartFolder::from_row)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        }
    })
    .await
}

pub async fn db_get_smart_folder_by_id(
    db: &DbState,
    id: String,
) -> Result<Option<DbSmartFolder>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM smart_folders WHERE id = ?1")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![id], DbSmartFolder::from_row)
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(folder)) => Ok(Some(folder)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

pub async fn db_insert_smart_folder(
    db: &DbState,
    id: String,
    name: String,
    query: String,
    account_id: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO smart_folders (id, account_id, name, query, icon, color)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                account_id,
                name,
                query,
                icon.unwrap_or_else(|| "search".to_owned()),
                color
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_smart_folder(
    db: &DbState,
    id: String,
    name: Option<String>,
    query: Option<String>,
    icon: Option<String>,
    color: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = query {
            sets.push(("query", Box::new(v)));
        }
        if let Some(v) = icon {
            sets.push(("icon", Box::new(v)));
        }
        if let Some(v) = color {
            sets.push(("color", Box::new(v)));
        }
        dynamic_update(conn, "smart_folders", "id", &id, sets)
    })
    .await
}

pub async fn db_delete_smart_folder(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM smart_folders WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_smart_folder_sort_order(
    db: &DbState,
    orders: Vec<SortOrderItem>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        for item in &orders {
            conn.execute(
                "UPDATE smart_folders SET sort_order = ?1 WHERE id = ?2",
                params![item.sort_order, item.id],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
}

pub async fn db_get_smart_label_rules_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbSmartLabelRule>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM smart_label_rules WHERE account_id = ?1
                     ORDER BY sort_order, created_at",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbSmartLabelRule::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_insert_smart_label_rule(
    db: &DbState,
    id: String,
    account_id: String,
    label_id: String,
    ai_description: String,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO smart_label_rules (id, account_id, label_id, ai_description, criteria_json, is_enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                account_id,
                label_id,
                ai_description,
                criteria_json,
                is_enabled.unwrap_or(true)
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_smart_label_rule(
    db: &DbState,
    id: String,
    label_id: Option<String>,
    ai_description: Option<String>,
    criteria_json: Option<String>,
    is_enabled: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = label_id {
            sets.push(("label_id", Box::new(v)));
        }
        if let Some(v) = ai_description {
            sets.push(("ai_description", Box::new(v)));
        }
        if let Some(v) = criteria_json {
            sets.push(("criteria_json", Box::new(v)));
        }
        if let Some(v) = is_enabled {
            sets.push(("is_enabled", Box::new(v)));
        }
        dynamic_update(conn, "smart_label_rules", "id", &id, sets)
    })
    .await
}

pub async fn db_delete_smart_label_rule(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM smart_label_rules WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
