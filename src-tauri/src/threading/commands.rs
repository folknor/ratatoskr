use serde::Deserialize;

use super::{ThreadGroup, ThreadableMessage, build_threads, update_threads};

#[derive(Deserialize)]
pub struct ThreadableMessageInput {
    pub id: String,
    pub message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub subject: Option<String>,
    pub date: i64,
}

#[derive(Deserialize)]
pub struct ThreadGroupInput {
    pub thread_id: String,
    pub message_ids: Vec<String>,
}

impl From<&ThreadableMessageInput> for ThreadableMessage {
    fn from(input: &ThreadableMessageInput) -> Self {
        Self {
            id: input.id.clone(),
            message_id: input.message_id.clone(),
            in_reply_to: input.in_reply_to.clone(),
            references: input.references.clone(),
            subject: input.subject.clone(),
            date: input.date,
        }
    }
}

impl From<&ThreadGroupInput> for ThreadGroup {
    fn from(input: &ThreadGroupInput) -> Self {
        Self {
            thread_id: input.thread_id.clone(),
            message_ids: input.message_ids.clone(),
        }
    }
}

/// Build threads from a list of messages using the JWZ algorithm.
/// Pure computation — no DB access.
#[tauri::command]
pub async fn threading_build_threads(
    messages: Vec<ThreadableMessageInput>,
) -> Result<Vec<ThreadGroup>, String> {
    // Offload to blocking thread since this can be CPU-intensive for large mailboxes
    tokio::task::spawn_blocking(move || {
        let msgs: Vec<ThreadableMessage> = messages.iter().map(Into::into).collect();
        Ok(build_threads(&msgs))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}

/// Incrementally update thread assignments given existing threads and new messages.
/// Pure computation — no DB access.
#[tauri::command]
pub async fn threading_update_threads(
    existing_threads: Vec<ThreadGroupInput>,
    new_messages: Vec<ThreadableMessageInput>,
) -> Result<Vec<ThreadGroup>, String> {
    tokio::task::spawn_blocking(move || {
        let existing: Vec<ThreadGroup> = existing_threads.iter().map(Into::into).collect();
        let new_msgs: Vec<ThreadableMessage> = new_messages.iter().map(Into::into).collect();
        Ok(update_threads(&existing, &new_msgs))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}
