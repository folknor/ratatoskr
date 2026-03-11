pub mod commands;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedSmartLabelMatch {
    pub thread_id: String,
    pub label_ids: Vec<String>,
}
