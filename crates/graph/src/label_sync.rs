use serde::Deserialize;

use super::client::GraphClient;
use db::db::DbState;
use db::db::queries_extra::{LabelWriteRow, upsert_labels};
use label_colors::preset_colors;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutlookCategory {
    id: String,
    display_name: String,
    color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CategoryListResponse {
    value: Vec<OutlookCategory>,
}

/// Sync the Exchange master category list from Graph API into the unified
/// labels system.
///
/// Fetches `GET /me/outlook/masterCategories` and upserts into the `labels`
/// table as `label_kind = 'tag'` entries with a `cat:` ID prefix.
pub async fn graph_label_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
) -> Result<usize, String> {
    let response: CategoryListResponse =
        client.get_json("/me/outlook/masterCategories", db).await?;

    let aid = account_id.to_string();
    let categories = response.value;
    let count = categories.len();
    log::info!("[Graph] Label sync for account {account_id}: {count} categories fetched");

    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("label sync tx: {e}"))?;

        let rows: Vec<LabelWriteRow> = categories
            .iter()
            .enumerate()
            .map(|(i, cat)| {
                let color_preset = cat.color.as_deref().unwrap_or("None");
                let (color_bg, color_fg) = if color_preset == "None" {
                    (None, None)
                } else {
                    match preset_colors::preset_to_hex(color_preset) {
                        Some((bg, fg)) => (Some(bg.to_string()), Some(fg.to_string())),
                        None => (None, None),
                    }
                };

                LabelWriteRow {
                    id: format!("cat:{}", cat.display_name),
                    account_id: aid.clone(),
                    name: cat.display_name.clone(),
                    label_type: "user".to_string(),
                    label_kind: "tag".to_string(),
                    color_bg,
                    color_fg,
                    sort_order: Some(i64::try_from(i).unwrap_or(0)),
                    imap_folder_path: None,
                    imap_special_use: None,
                    parent_label_id: None,
                    right_read: None,
                    right_add: None,
                    right_remove: None,
                    right_set_seen: None,
                    right_set_keywords: None,
                    right_create_child: None,
                    right_rename: None,
                    right_delete: None,
                    right_submit: None,
                    is_subscribed: None,
                }
            })
            .collect();

        upsert_labels(&tx, &rows)?;

        tx.commit()
            .map_err(|e| format!("label sync commit: {e}"))?;
        Ok(count)
    })
    .await
}
