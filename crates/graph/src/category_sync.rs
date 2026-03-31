use serde::Deserialize;

use super::client::GraphClient;
use db::db::DbState;
use label_colors::category_colors;

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

        for (i, cat) in categories.iter().enumerate() {
            let color_preset = cat.color.as_deref().unwrap_or("None");
            let (color_bg, color_fg) = if color_preset == "None" {
                (None, None)
            } else {
                match category_colors::preset_to_hex(color_preset) {
                    Some((bg, fg)) => (Some(bg), Some(fg)),
                    None => (None, None),
                }
            };

            let label_id = format!("cat:{}", cat.display_name);
            tx.execute(
                "INSERT INTO labels (id, account_id, name, type, label_kind, color_bg, color_fg, sort_order)
                 VALUES (?1, ?2, ?3, 'user', 'tag', ?4, ?5, ?6)
                 ON CONFLICT (account_id, id) DO UPDATE SET
                     name = excluded.name,
                     color_bg = excluded.color_bg,
                     color_fg = excluded.color_fg,
                     sort_order = excluded.sort_order",
                rusqlite::params![
                    label_id,
                    aid,
                    cat.display_name,
                    color_bg,
                    color_fg,
                    i64::try_from(i).unwrap_or(0),
                ],
            )
            .map_err(|e| format!("upsert category label: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("label sync commit: {e}"))?;
        Ok(count)
    })
    .await
}
