use std::collections::HashMap;

use crate::db::ReadConn;

const MAX_PAIRS_PER_CHUNK: usize = 256;

#[derive(Debug, Clone)]
pub struct AttachmentFragmentRow {
    pub attachment_id: String,
    pub message_id: String,
    pub account_id: String,
    pub filename: String,
    pub mime_type: String,
    pub extracted_text: String,
}

pub fn select_attachment_fragments_batch(
    conn: &ReadConn<'_>,
    pairs: &[(String, String)],
) -> Result<HashMap<(String, String), Vec<AttachmentFragmentRow>>, String> {
    let mut out: HashMap<(String, String), Vec<AttachmentFragmentRow>> = HashMap::new();
    if pairs.is_empty() {
        return Ok(out);
    }
    for chunk in pairs.chunks(MAX_PAIRS_PER_CHUNK) {
        let placeholders: Vec<String> = (0..chunk.len())
            .map(|i| format!("(?{}, ?{})", i * 2 + 1, i * 2 + 2))
            .collect();
        let sql = format!(
            "SELECT a.id, a.message_id, a.account_id, a.filename, a.mime_type,
                    t.extracted_text, t.status
             FROM attachments a
             LEFT JOIN attachment_extracted_text t ON t.content_hash = a.content_hash
             WHERE (a.account_id, a.message_id) IN (VALUES {})
             ORDER BY a.account_id, a.message_id, a.rowid",
            placeholders.join(", "),
        );
        let mut params_vec: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() * 2);
        for (acc, mid) in chunk {
            params_vec.push(acc);
            params_vec.push(mid);
        }
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare select_attachment_fragments_batch: {e}"))?;
        let rows = stmt
            .query_map(params_vec.as_slice(), |row| {
                let attachment_id = row.get::<_, String>(0)?;
                let message_id = row.get::<_, String>(1)?;
                let account_id = row.get::<_, String>(2)?;
                let filename = row.get::<_, Option<String>>(3)?.unwrap_or_default();
                let mime_type = row.get::<_, Option<String>>(4)?.unwrap_or_default();
                let extracted_text = row.get::<_, Option<String>>(5)?;
                let status = row.get::<_, Option<String>>(6)?;
                let text = match (extracted_text, status.as_deref()) {
                    (Some(t), Some("indexed")) => t,
                    _ => String::new(),
                };
                Ok(AttachmentFragmentRow {
                    attachment_id,
                    message_id,
                    account_id,
                    filename,
                    mime_type,
                    extracted_text: text,
                })
            })
            .map_err(|e| format!("query select_attachment_fragments_batch: {e}"))?;
        for r in rows {
            let frag = r.map_err(|e| format!("row select_attachment_fragments_batch: {e}"))?;
            out.entry((frag.account_id.clone(), frag.message_id.clone()))
                .or_default()
                .push(frag);
        }
    }
    Ok(out)
}
