pub mod commands;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedSmartLabelMatch {
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLabelAIThread {
    pub id: String,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLabelAIRule {
    pub label_id: String,
    pub description: String,
}
