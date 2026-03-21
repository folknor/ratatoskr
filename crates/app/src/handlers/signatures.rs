use std::sync::Arc;

use iced::Task;

use crate::ui::settings::SignatureSaveRequest;
use crate::{App, Message};

impl App {
    pub(crate) fn handle_save_signature(&mut self, req: SignatureSaveRequest) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    if let Some(ref id) = req.id {
                        conn.execute(
                            "UPDATE signatures SET name = ?1, body_html = ?2, \
                             is_default = ?3, is_reply_default = ?4 WHERE id = ?5",
                            rusqlite::params![
                                req.name,
                                req.body_html,
                                if req.is_default { 1 } else { 0 },
                                if req.is_reply_default { 1 } else { 0 },
                                id,
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    } else {
                        let id = uuid::Uuid::new_v4().to_string();
                        conn.execute(
                            "INSERT INTO signatures (id, account_id, name, body_html, \
                             is_default, is_reply_default) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            rusqlite::params![
                                id,
                                req.account_id,
                                req.name,
                                req.body_html,
                                if req.is_default { 1 } else { 0 },
                                if req.is_reply_default { 1 } else { 0 },
                            ],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    eprintln!("Failed to save signature: {e}");
                }
                Message::ReloadSignatures
            },
        )
    }

    pub(crate) fn handle_delete_signature(&mut self, sig_id: String) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    conn.execute(
                        "DELETE FROM signatures WHERE id = ?1",
                        rusqlite::params![sig_id],
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(())
                })
                .await
            },
            |result| {
                if let Err(e) = result {
                    eprintln!("Failed to delete signature: {e}");
                }
                Message::ReloadSignatures
            },
        )
    }

    /// Load signatures from DB into settings.signatures synchronously.
    pub(crate) fn load_signatures_into_settings(&mut self) {
        let result = self.db.with_conn_sync(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, is_default,
                            is_reply_default, sort_order
                     FROM signatures ORDER BY account_id, sort_order, name",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(crate::ui::settings::SignatureEntry {
                        id: row.get("id")?,
                        account_id: row.get("account_id")?,
                        name: row.get("name")?,
                        body_html: row.get::<_, Option<String>>("body_html")?
                            .unwrap_or_default(),
                        body_text: None,
                        is_default: row.get::<_, i64>("is_default").unwrap_or(0) != 0,
                        is_reply_default: row.get::<_, i64>("is_reply_default")
                            .unwrap_or(0)
                            != 0,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(sigs) => self.settings.signatures = sigs,
            Err(e) => eprintln!("Failed to load signatures: {e}"),
        }
    }
}
