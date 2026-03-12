use serde_json::json;

use crate::db::DbState;

use super::client::GmailClient;
use super::types::{
    GmailAttachmentData, GmailDraft, GmailDraftStub, GmailHistoryResponse, GmailLabel,
    GmailMessage, GmailProfile, GmailSendAs, GmailThread, GmailThreadStub, ListDraftsResponse,
    ListLabelsResponse, ListSendAsResponse, ListThreadsResponse,
};

// ── Profile ─────────────────────────────────────────────────

impl GmailClient {
    pub async fn get_profile(&self, db: &DbState) -> Result<GmailProfile, String> {
        self.get("/profile", db).await
    }
}

// ── Labels ──────────────────────────────────────────────────

impl GmailClient {
    pub async fn list_labels(&self, db: &DbState) -> Result<Vec<GmailLabel>, String> {
        let resp: ListLabelsResponse = self.get("/labels", db).await?;
        Ok(resp.labels)
    }

    pub async fn create_label(
        &self,
        name: &str,
        color: Option<(&str, &str)>,
        db: &DbState,
    ) -> Result<GmailLabel, String> {
        let mut body = json!({
            "name": name,
            "labelListVisibility": "labelShow",
            "messageListVisibility": "show",
        });
        if let Some((text_color, bg_color)) = color {
            body["color"] = json!({
                "textColor": text_color,
                "backgroundColor": bg_color,
            });
        }
        self.post("/labels", &body, db).await
    }

    pub async fn update_label(
        &self,
        label_id: &str,
        name: Option<&str>,
        color: Option<Option<(&str, &str)>>,
        db: &DbState,
    ) -> Result<GmailLabel, String> {
        let mut body = json!({});
        if let Some(n) = name {
            body["name"] = json!(n);
        }
        if let Some(c) = color {
            body["color"] = match c {
                Some((text, bg)) => json!({"textColor": text, "backgroundColor": bg}),
                None => serde_json::Value::Null,
            };
        }
        self.patch(&format!("/labels/{label_id}"), &body, db).await
    }

    pub async fn delete_label(&self, label_id: &str, db: &DbState) -> Result<(), String> {
        self.delete(&format!("/labels/{label_id}"), db).await
    }
}

// ── Threads ─────────────────────────────────────────────────

impl GmailClient {
    pub async fn list_threads(
        &self,
        query: Option<&str>,
        max_results: Option<u32>,
        page_token: Option<&str>,
        db: &DbState,
    ) -> Result<(Vec<GmailThreadStub>, Option<String>), String> {
        let mut params = Vec::new();
        if let Some(q) = query {
            params.push(format!("q={}", urlencoding::encode(q)));
        }
        if let Some(max) = max_results {
            params.push(format!("maxResults={max}"));
        }
        if let Some(pt) = page_token {
            params.push(format!("pageToken={pt}"));
        }
        let qs = if params.is_empty() {
            String::new()
        } else {
            format!("?{}", params.join("&"))
        };

        let resp: ListThreadsResponse = self.get(&format!("/threads{qs}"), db).await?;
        Ok((resp.threads, resp.next_page_token))
    }

    pub async fn get_thread(
        &self,
        thread_id: &str,
        format: &str,
        db: &DbState,
    ) -> Result<GmailThread, String> {
        self.get(&format!("/threads/{thread_id}?format={format}"), db)
            .await
    }

    /// Modify labels on all messages in a thread.
    pub async fn modify_thread(
        &self,
        thread_id: &str,
        add_labels: &[String],
        remove_labels: &[String],
        db: &DbState,
    ) -> Result<GmailThread, String> {
        self.post(
            &format!("/threads/{thread_id}/modify"),
            &json!({
                "addLabelIds": add_labels,
                "removeLabelIds": remove_labels,
            }),
            db,
        )
        .await
    }

    /// Permanently delete a thread (cannot be undone).
    pub async fn delete_thread(&self, thread_id: &str, db: &DbState) -> Result<(), String> {
        self.delete(&format!("/threads/{thread_id}"), db).await
    }
}

// ── Messages ────────────────────────────────────────────────

impl GmailClient {
    pub async fn get_message(
        &self,
        message_id: &str,
        format: &str,
        db: &DbState,
    ) -> Result<GmailMessage, String> {
        self.get(&format!("/messages/{message_id}?format={format}"), db)
            .await
    }

    /// Send an email. `raw` is base64url-encoded RFC 2822 message.
    pub async fn send_message(
        &self,
        raw: &str,
        thread_id: Option<&str>,
        db: &DbState,
    ) -> Result<GmailMessage, String> {
        let mut body = json!({ "raw": raw });
        if let Some(tid) = thread_id {
            body["threadId"] = json!(tid);
        }
        self.post("/messages/send", &body, db).await
    }

    pub async fn get_attachment(
        &self,
        message_id: &str,
        attachment_id: &str,
        db: &DbState,
    ) -> Result<GmailAttachmentData, String> {
        self.get(
            &format!("/messages/{message_id}/attachments/{attachment_id}"),
            db,
        )
        .await
    }
}

// ── History ─────────────────────────────────────────────────

impl GmailClient {
    pub async fn get_history(
        &self,
        start_history_id: &str,
        page_token: Option<&str>,
        db: &DbState,
    ) -> Result<GmailHistoryResponse, String> {
        let mut params = vec![
            format!("startHistoryId={start_history_id}"),
            "historyTypes=messageAdded".to_string(),
            "historyTypes=messageDeleted".to_string(),
            "historyTypes=labelAdded".to_string(),
            "historyTypes=labelRemoved".to_string(),
        ];
        if let Some(pt) = page_token {
            params.push(format!("pageToken={pt}"));
        }
        let qs = params.join("&");
        self.get(&format!("/history?{qs}"), db).await
    }
}

// ── Drafts ──────────────────────────────────────────────────

impl GmailClient {
    pub async fn create_draft(
        &self,
        raw: &str,
        thread_id: Option<&str>,
        db: &DbState,
    ) -> Result<GmailDraft, String> {
        let mut message = json!({ "raw": raw });
        if let Some(tid) = thread_id {
            message["threadId"] = json!(tid);
        }
        self.post("/drafts", &json!({ "message": message }), db)
            .await
    }

    pub async fn update_draft(
        &self,
        draft_id: &str,
        raw: &str,
        thread_id: Option<&str>,
        db: &DbState,
    ) -> Result<GmailDraft, String> {
        let mut message = json!({ "raw": raw });
        if let Some(tid) = thread_id {
            message["threadId"] = json!(tid);
        }
        self.put(
            &format!("/drafts/{draft_id}"),
            &json!({ "message": message }),
            db,
        )
        .await
    }

    pub async fn delete_draft(&self, draft_id: &str, db: &DbState) -> Result<(), String> {
        self.delete(&format!("/drafts/{draft_id}"), db).await
    }

    pub async fn list_drafts(&self, db: &DbState) -> Result<Vec<GmailDraftStub>, String> {
        let resp: ListDraftsResponse = self.get("/drafts?maxResults=500", db).await?;
        Ok(resp.drafts)
    }
}

// ── Send-as ─────────────────────────────────────────────────

impl GmailClient {
    pub async fn list_send_as(&self, db: &DbState) -> Result<Vec<GmailSendAs>, String> {
        let resp: ListSendAsResponse = self.get("/settings/sendAs", db).await?;
        Ok(resp.send_as)
    }
}
