use serde::Deserialize;

use super::client::GraphClient;
use ratatoskr_category_colors as category_colors;
use ratatoskr_db::db::DbState;

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

/// Sync the Exchange master category list from Graph API.
///
/// Fetches `GET /me/outlook/masterCategories` and upserts into the
/// `categories` table. Categories removed from the server are marked
/// but not deleted locally (they may still be referenced by messages).
pub async fn graph_categories_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
) -> Result<usize, String> {
    let response: CategoryListResponse = client
        .get_json("/me/outlook/masterCategories", db)
        .await?;

    let aid = account_id.to_string();
    let categories = response.value;
    let count = categories.len();

    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("category sync tx: {e}"))?;

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

            tx.execute(
                "INSERT INTO categories \
                 (id, account_id, display_name, color_preset, color_bg, color_fg, \
                  provider_id, sync_state, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'synced', ?8) \
                 ON CONFLICT(account_id, display_name) DO UPDATE SET \
                   color_preset = ?4, color_bg = ?5, color_fg = ?6, \
                   provider_id = ?7, sync_state = 'synced', sort_order = ?8",
                rusqlite::params![
                    cat.id,
                    aid,
                    cat.display_name,
                    color_preset,
                    color_bg,
                    color_fg,
                    cat.id,
                    i as i64,
                ],
            )
            .map_err(|e| format!("upsert category: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("category sync commit: {e}"))?;
        Ok(count)
    })
    .await
}
