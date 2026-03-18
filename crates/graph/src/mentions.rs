use serde::Deserialize;

use ratatoskr_db::db::DbState;

use super::client::GraphClient;

const GRAPH_BETA_BASE: &str = "https://graph.microsoft.com/beta";

/// A single mention entry from the Graph beta API `$expand=mentions` response.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMention {
    id: Option<String>,
    mentioned: Option<GraphMentionPerson>,
    created_by: Option<GraphMentionPerson>,
    created_date_time: Option<String>,
}

/// An email address + display name pair within a mention.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMentionPerson {
    name: Option<String>,
    address: Option<String>,
}

/// The top-level response from `GET /beta/me/messages/{id}?$expand=mentions`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphMessageWithMentions {
    #[serde(default)]
    mentions: Vec<GraphMention>,
}

/// A mention record returned to callers after fetching and storing.
#[derive(Debug, Clone)]
pub struct Mention {
    pub mention_id: Option<String>,
    pub mentioned_name: Option<String>,
    pub mentioned_address: String,
    pub created_by_name: Option<String>,
    pub created_by_address: Option<String>,
    pub created_at: Option<i64>,
}

/// Fetch full mention details from the Graph beta API and upsert them into the
/// local `mentions` table.
///
/// Call this when a message with `is_mentioned = 1` is opened to lazy-load the
/// mention participants (who mentioned whom).
pub async fn fetch_and_store_mentions(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
    message_id: &str,
) -> Result<Vec<Mention>, String> {
    let enc_id = urlencoding::encode(message_id);
    let url = format!("{GRAPH_BETA_BASE}/me/messages/{enc_id}?$expand=mentions");

    let response: GraphMessageWithMentions = client.get_absolute(&url, db).await?;

    let mentions: Vec<Mention> = response
        .mentions
        .into_iter()
        .filter_map(|gm| {
            let mentioned_address = gm.mentioned.as_ref()?.address.clone()?;
            Some(Mention {
                mention_id: gm.id,
                mentioned_name: gm.mentioned.as_ref().and_then(|m| m.name.clone()),
                mentioned_address,
                created_by_name: gm.created_by.as_ref().and_then(|c| c.name.clone()),
                created_by_address: gm.created_by.as_ref().and_then(|c| c.address.clone()),
                created_at: gm.created_date_time.as_ref().and_then(|dt| parse_iso8601(dt)),
            })
        })
        .collect();

    // Upsert into DB
    let aid = account_id.to_string();
    let mid = message_id.to_string();
    let mentions_clone = mentions.clone();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "INSERT INTO mentions \
                 (message_id, account_id, mention_id, mentioned_name, mentioned_address, \
                  created_by_name, created_by_address, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(message_id, account_id, mentioned_address) DO UPDATE SET \
                   mention_id = excluded.mention_id, \
                   mentioned_name = excluded.mentioned_name, \
                   created_by_name = excluded.created_by_name, \
                   created_by_address = excluded.created_by_address, \
                   created_at = excluded.created_at",
            )
            .map_err(|e| format!("prepare mention upsert: {e}"))?;

        for m in &mentions_clone {
            stmt.execute(rusqlite::params![
                mid,
                aid,
                m.mention_id,
                m.mentioned_name,
                m.mentioned_address,
                m.created_by_name,
                m.created_by_address,
                m.created_at,
            ])
            .map_err(|e| format!("upsert mention: {e}"))?;
        }

        Ok(())
    })
    .await?;

    log::info!(
        "Fetched and stored {} mentions for message {message_id}",
        mentions.len()
    );

    Ok(mentions)
}

/// Parse an ISO 8601 datetime string to a Unix timestamp (seconds).
fn parse_iso8601(s: &str) -> Option<i64> {
    // Handle common formats: "2026-03-14T10:00:00Z" and "2026-03-14T10:00:00.0000000Z"
    let trimmed = s.trim_end_matches('Z');
    // Split off fractional seconds if present
    let without_frac = if let Some(dot_pos) = trimmed.rfind('.') {
        &trimmed[..dot_pos]
    } else {
        trimmed
    };

    // Parse "YYYY-MM-DDTHH:MM:SS"
    let parts: Vec<&str> = without_frac.split('T').collect();
    if parts.len() != 2 {
        return None;
    }

    let date_parts: Vec<&str> = parts[0].split('-').collect();
    let time_parts: Vec<&str> = parts[1].split(':').collect();

    if date_parts.len() != 3 || time_parts.len() != 3 {
        return None;
    }

    let year: i32 = date_parts[0].parse().ok()?;
    let month: u32 = date_parts[1].parse().ok()?;
    let day: u32 = date_parts[2].parse().ok()?;
    let hour: u32 = time_parts[0].parse().ok()?;
    let min: u32 = time_parts[1].parse().ok()?;
    let sec: u32 = time_parts[2].parse().ok()?;

    // Convert to Unix timestamp using a simplified calculation (UTC only).
    // Days from year 1970 to the given date.
    fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y } as i64;
        let m = m as i64;
        let d = d as i64;
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86400 + i64::from(hour) * 3600 + i64::from(min) * 60 + i64::from(sec))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso8601_basic() {
        let ts = parse_iso8601("2026-03-14T10:00:00Z");
        assert!(ts.is_some());
        // Just verify it's a reasonable value (after 2025)
        assert!(ts.unwrap() > 1_700_000_000);
    }

    #[test]
    fn parse_iso8601_with_fractional() {
        let ts = parse_iso8601("2026-03-14T10:00:00.0000000Z");
        assert!(ts.is_some());
        assert!(ts.unwrap() > 1_700_000_000);
    }

    #[test]
    fn parse_iso8601_invalid() {
        assert!(parse_iso8601("not-a-date").is_none());
        assert!(parse_iso8601("").is_none());
    }
}
