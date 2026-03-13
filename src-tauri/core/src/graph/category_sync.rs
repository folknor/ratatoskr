use serde::Deserialize;

use super::client::GraphClient;
use crate::db::DbState;

/// Exchange category color preset → (background hex, foreground hex).
///
/// Colors are based on the documented Outlook preset names. Hex values
/// approximate what Outlook for Windows renders for each preset.
const PRESET_COLORS: &[(&str, &str, &str)] = &[
    ("preset0", "#e74c3c", "#ffffff"),  // Red
    ("preset1", "#e67e22", "#ffffff"),  // Orange
    ("preset2", "#8b4513", "#ffffff"),  // Brown
    ("preset3", "#f1c40f", "#000000"),  // Yellow
    ("preset4", "#2ecc71", "#ffffff"),  // Green
    ("preset5", "#1abc9c", "#ffffff"),  // Teal
    ("preset6", "#808000", "#ffffff"),  // Olive
    ("preset7", "#3498db", "#ffffff"),  // Blue
    ("preset8", "#9b59b6", "#ffffff"),  // Purple
    ("preset9", "#c0392b", "#ffffff"),  // Cranberry
    ("preset10", "#708090", "#ffffff"), // Steel
    ("preset11", "#4a5568", "#ffffff"), // DarkSteel
    ("preset12", "#95a5a6", "#000000"), // Gray
    ("preset13", "#636e72", "#ffffff"), // DarkGray
    ("preset14", "#2d3436", "#ffffff"), // Black
    ("preset15", "#8b0000", "#ffffff"), // DarkRed
    ("preset16", "#d35400", "#ffffff"), // DarkOrange
    ("preset17", "#5d3a1a", "#ffffff"), // DarkBrown
    ("preset18", "#b8860b", "#ffffff"), // DarkYellow
    ("preset19", "#1e7e34", "#ffffff"), // DarkGreen
    ("preset20", "#0e6655", "#ffffff"), // DarkTeal
    ("preset21", "#556b2f", "#ffffff"), // DarkOlive
    ("preset22", "#1a5276", "#ffffff"), // DarkBlue
    ("preset23", "#6c3483", "#ffffff"), // DarkPurple
    ("preset24", "#922b21", "#ffffff"), // DarkCranberry
];

fn preset_to_colors(preset: &str) -> (Option<&'static str>, Option<&'static str>) {
    PRESET_COLORS
        .iter()
        .find(|(name, _, _)| *name == preset)
        .map(|(_, bg, fg)| (Some(*bg), Some(*fg)))
        .unwrap_or((None, None))
}

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
                preset_to_colors(color_preset)
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
