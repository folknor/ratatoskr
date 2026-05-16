use serde::Deserialize;

use super::client::GraphClient;
use db::db::ReadDbState;
use db::db::queries_extra::{LabelWriteRow, upsert_labels};
use label_colors::preset_colors;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // id deserialized from API for diagnostics; not currently used downstream
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
/// table entries with a `cat:` ID prefix.
pub async fn graph_label_sync(
    client: &GraphClient,
    account_id: &str,
    db: &ReadDbState,
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

        let mut rows: Vec<LabelWriteRow> = categories
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
                    visible: None,
                    sort_order: Some(i64::try_from(i).unwrap_or(0)),
                    server_color_bg: color_bg,
                    server_color_fg: color_fg,
                    user_color_bg: None,
                    user_color_fg: None,
                    is_undeletable: false,
                }
            })
            .collect();
        rows.extend(importance_label_rows(&aid));

        upsert_labels(&tx, &rows)?;

        tx.commit()
            .map_err(|e| format!("label sync commit: {e}"))?;
        Ok(count)
    })
    .await
}

fn importance_label_rows(account_id: &str) -> Vec<LabelWriteRow> {
    [
        ("importance:high", "High importance", 10_000),
        ("importance:low", "Low importance", 10_001),
    ]
    .into_iter()
    .map(|(id, name, sort_order)| LabelWriteRow {
        id: id.to_string(),
        account_id: account_id.to_string(),
        name: name.to_string(),
        visible: None,
        sort_order: Some(sort_order),
        server_color_bg: None,
        server_color_fg: None,
        user_color_bg: None,
        user_color_fg: None,
        is_undeletable: true,
    })
    .collect()
}
