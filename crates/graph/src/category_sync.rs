use serde::Deserialize;

use super::client::GraphClient;
use ratatoskr_label_colors::category_colors;
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

            ratatoskr_db::db::queries::upsert_category(
                &tx,
                &cat.id,
                &aid,
                &cat.display_name,
                &ratatoskr_db::db::queries::CategoryColors {
                    preset: Some(color_preset),
                    bg: color_bg,
                    fg: color_fg,
                },
                &cat.id,
                i as i64,
                true,
                ratatoskr_db::db::queries::CategorySortOnConflict::Update,
            )?;
        }

        tx.commit()
            .map_err(|e| format!("category sync commit: {e}"))?;
        Ok(count)
    })
    .await
}
