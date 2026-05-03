use std::sync::Arc;

use iced::Task;

use crate::db::Db;
use crate::pop_out::message_view::MessageViewMessage;
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Handle Save As action from the overflow menu.
    pub(crate) fn handle_save_as(&self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get(&window_id) else {
            return Task::none();
        };

        let db = Arc::clone(&self.db);
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();
        let subject = state
            .subject
            .clone()
            .unwrap_or_else(|| "message".to_string());

        Task::perform(
            async move { save_message_dialog(db, account_id, message_id, subject).await },
            move |_result| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::MessageView(MessageViewMessage::Noop),
                )
            },
        )
    }
}

/// Sanitize a subject line for use as a filename.
fn sanitize_filename(subject: &str) -> String {
    let safe: String = subject
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim().to_string();
    if trimmed.is_empty() {
        "message".to_string()
    } else {
        trimmed
    }
}

/// Open a native file picker and save the message as .eml or .txt.
async fn save_message_dialog(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    subject: String,
) -> Result<(), String> {
    let safe_name = sanitize_filename(&subject);

    let file_handle = rfd::AsyncFileDialog::new()
        .set_title("Save Message As")
        .set_file_name(format!("{safe_name}.eml"))
        .add_filter("Email Message (.eml)", &["eml"])
        .add_filter("Plain Text (.txt)", &["txt"])
        .save_file()
        .await;

    let Some(handle) = file_handle else {
        return Ok(()); // user cancelled
    };

    let path = handle.path().to_path_buf();
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("eml");

    match extension {
        "txt" => {
            let (body_text, _body_html) = db.load_message_body(account_id, message_id).await?;
            let txt_content = body_text.unwrap_or_default();
            std::fs::write(&path, txt_content.as_bytes())
                .map_err(|e| format!("Write failed: {e}"))?;
        }
        _ => {
            // Default to .eml
            let raw = db.load_raw_source(account_id, message_id).await?;
            std::fs::write(&path, raw.as_bytes()).map_err(|e| format!("Write failed: {e}"))?;
        }
    }

    Ok(())
}
