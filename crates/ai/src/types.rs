use serde::{Deserialize, Serialize};
use std::fmt;

// Re-export ThreadBundle from its canonical location in core/sync.
pub use rtsk::bundling::ThreadBundle;

/// Request sent to an AI completion provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCompletionRequest {
    pub system_prompt: String,
    pub user_content: String,
    pub max_tokens: Option<u32>,
}

/// Supported AI providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProvider {
    Claude,
    OpenAi,
    Gemini,
    Ollama,
    Copilot,
}

impl AiProvider {
    /// Parse a provider from its canonical string name.
    pub fn from_str_name(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "openai" => Some(Self::OpenAi),
            "gemini" => Some(Self::Gemini),
            "ollama" => Some(Self::Ollama),
            "copilot" => Some(Self::Copilot),
            _ => None,
        }
    }

    /// Return the canonical string name for this provider.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenAi => "openai",
            Self::Gemini => "gemini",
            Self::Ollama => "ollama",
            Self::Copilot => "copilot",
        }
    }

    /// Return the default model ID for this provider.
    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Claude => "claude-haiku-4-5-20251001",
            Self::OpenAi => "gpt-4o-mini",
            Self::Gemini => "gemini-2.5-flash-preview-05-20",
            Self::Ollama => "llama3.2",
            Self::Copilot => "openai/gpt-4o-mini",
        }
    }
}

impl fmt::Display for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// AI provider configuration.
#[derive(Debug, Clone)]
pub struct AiConfig {
    pub provider: AiProvider,
    pub api_key: Option<String>,
    pub model: String,
    /// Server URL (used for Ollama).
    pub server_url: Option<String>,
}

/// Errors that can occur during AI operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// Provider is not configured (missing API key, server URL, etc.).
    NotConfigured(String),
    /// Authentication failed (invalid API key).
    AuthError(String),
    /// Rate limited by the provider.
    RateLimited(String),
    /// Network or HTTP error.
    NetworkError(String),
    /// Failed to parse the AI response into the expected format.
    ParseError(String),
    /// Database operation failed.
    DbError(String),
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured(msg) => write!(f, "NOT_CONFIGURED: {msg}"),
            Self::AuthError(msg) => write!(f, "AUTH_ERROR: {msg}"),
            Self::RateLimited(msg) => write!(f, "RATE_LIMITED: {msg}"),
            Self::NetworkError(msg) => write!(f, "NETWORK_ERROR: {msg}"),
            Self::ParseError(msg) => write!(f, "AI response parse error: {msg}"),
            Self::DbError(msg) => write!(f, "AI database error: {msg}"),
        }
    }
}

impl std::error::Error for AiError {}

impl AiError {
    /// Classify an HTTP error response into the appropriate `AiError` variant.
    pub fn from_http(status: u16, body: &str) -> Self {
        let lower = body.to_lowercase();
        if status == 401
            || status == 403
            || lower.contains("invalid api key")
            || lower.contains("unauthorized")
            || lower.contains("authentication")
        {
            return Self::AuthError("Invalid API key".to_string());
        }
        if status == 429
            || lower.contains("rate limit")
            || lower.contains("too many requests")
            || lower.contains("resource exhausted")
        {
            return Self::RateLimited("Rate limited — please try again shortly".to_string());
        }
        Self::NetworkError(format!("HTTP {status} {body}"))
    }
}

/// Trait for AI completion — allows testing with mock implementations.
/// The app crate provides the real implementation that calls AI providers.
#[async_trait::async_trait]
pub trait AiCompleter: Send + Sync {
    async fn complete(&self, request: &AiCompletionRequest) -> Result<String, AiError>;
}

/// Input message for AI operations (summary, smart replies, etc.).
#[derive(Debug, Clone)]
pub struct AiMessageInput {
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: i64,
    pub body_text: Option<String>,
    pub snippet: Option<String>,
    pub subject: Option<String>,
}

/// Search result passed to ask_inbox.
#[derive(Debug, Clone)]
pub struct AiSearchResult {
    pub message_id: String,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub snippet: Option<String>,
}

/// Thread data for AI bundling.
#[derive(Debug, Clone)]
pub struct ThreadForBundling {
    pub thread_id: String,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
}

/// Task extracted from an email thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTask {
    pub title: String,
    pub description: String,
    pub due_date: Option<i64>,
    pub priority: TaskPriority,
}

/// Task priority levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    None,
    Low,
    Medium,
    High,
    Urgent,
}

/// Text transform types for the transform_text function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextTransformType {
    Improve,
    Shorten,
    Formalize,
}

/// Writing style profile produced by analyze_writing_style.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WritingStyleProfile {
    pub profile_text: String,
    pub sample_count: i64,
}

/// Auto-draft mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDraftMode {
    Reply,
    ReplyAll,
}

impl AutoDraftMode {
    pub fn cache_type(&self) -> &'static str {
        match self {
            Self::Reply => "auto_draft_reply",
            Self::ReplyAll => "auto_draft_replyAll",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- AiProvider --

    #[test]
    fn ai_provider_roundtrip() {
        let providers = [
            AiProvider::Claude,
            AiProvider::OpenAi,
            AiProvider::Gemini,
            AiProvider::Ollama,
            AiProvider::Copilot,
        ];
        for provider in providers {
            let name = provider.as_str();
            let parsed = AiProvider::from_str_name(name);
            assert_eq!(parsed, Some(provider), "roundtrip failed for {name}");
        }
    }

    #[test]
    fn ai_provider_unknown_returns_none() {
        assert_eq!(AiProvider::from_str_name("unknown"), None);
        assert_eq!(AiProvider::from_str_name(""), None);
    }

    #[test]
    fn ai_provider_default_models() {
        assert_eq!(
            AiProvider::Claude.default_model(),
            "claude-haiku-4-5-20251001"
        );
        assert_eq!(AiProvider::OpenAi.default_model(), "gpt-4o-mini");
        assert_eq!(
            AiProvider::Gemini.default_model(),
            "gemini-2.5-flash-preview-05-20"
        );
        assert_eq!(AiProvider::Ollama.default_model(), "llama3.2");
        assert_eq!(AiProvider::Copilot.default_model(), "openai/gpt-4o-mini");
    }

    #[test]
    fn ai_provider_display() {
        assert_eq!(AiProvider::Claude.to_string(), "claude");
        assert_eq!(AiProvider::OpenAi.to_string(), "openai");
    }

    // -- TaskPriority --

    #[test]
    fn task_priority_serde_roundtrip() {
        let priorities = [
            TaskPriority::None,
            TaskPriority::Low,
            TaskPriority::Medium,
            TaskPriority::High,
            TaskPriority::Urgent,
        ];
        for p in priorities {
            let json = serde_json::to_string(&p).expect("serialize");
            let parsed: TaskPriority = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(parsed, p);
        }
    }

    #[test]
    fn task_priority_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&TaskPriority::High).expect("ok"),
            "\"high\""
        );
        assert_eq!(
            serde_json::to_string(&TaskPriority::None).expect("ok"),
            "\"none\""
        );
    }

    // -- ExtractedTask --

    #[test]
    fn extracted_task_deserialization() {
        let json = r#"{
            "title": "Review PR #42",
            "description": "John asked for review by Friday",
            "due_date": 1700000000,
            "priority": "high"
        }"#;
        let task: ExtractedTask = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(task.title, "Review PR #42");
        assert_eq!(task.description, "John asked for review by Friday");
        assert_eq!(task.due_date, Some(1700000000));
        assert_eq!(task.priority, TaskPriority::High);
    }

    #[test]
    fn extracted_task_deserialization_with_null_due_date() {
        let json = r#"{
            "title": "Follow up on: Weekly sync",
            "description": "",
            "due_date": null,
            "priority": "none"
        }"#;
        let task: ExtractedTask = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(task.title, "Follow up on: Weekly sync");
        assert!(task.due_date.is_none());
        assert_eq!(task.priority, TaskPriority::None);
    }

    // -- AiError --

    #[test]
    fn ai_error_from_http_auth() {
        let err = AiError::from_http(401, "Unauthorized");
        assert_eq!(err, AiError::AuthError("Invalid API key".to_string()));

        let err = AiError::from_http(200, "invalid api key provided");
        assert_eq!(err, AiError::AuthError("Invalid API key".to_string()));

        let err = AiError::from_http(403, "Forbidden");
        assert_eq!(err, AiError::AuthError("Invalid API key".to_string()));
    }

    #[test]
    fn ai_error_from_http_rate_limited() {
        let err = AiError::from_http(429, "Too Many Requests");
        assert_eq!(
            err,
            AiError::RateLimited("Rate limited — please try again shortly".to_string())
        );

        let err = AiError::from_http(200, "resource exhausted");
        assert_eq!(
            err,
            AiError::RateLimited("Rate limited — please try again shortly".to_string())
        );
    }

    #[test]
    fn ai_error_from_http_network() {
        let err = AiError::from_http(500, "Internal Server Error");
        assert_eq!(
            err,
            AiError::NetworkError("HTTP 500 Internal Server Error".to_string())
        );
    }

    #[test]
    fn ai_error_display() {
        assert_eq!(
            AiError::NotConfigured("no key".to_string()).to_string(),
            "NOT_CONFIGURED: no key"
        );
        assert_eq!(
            AiError::AuthError("bad key".to_string()).to_string(),
            "AUTH_ERROR: bad key"
        );
        assert_eq!(
            AiError::ParseError("bad json".to_string()).to_string(),
            "AI response parse error: bad json"
        );
        assert_eq!(
            AiError::DbError("connection lost".to_string()).to_string(),
            "AI database error: connection lost"
        );
    }

    // -- AiCompletionRequest --

    #[test]
    fn ai_completion_request_serialization() {
        let req = AiCompletionRequest {
            system_prompt: "You are helpful.".to_string(),
            user_content: "Hello".to_string(),
            max_tokens: Some(100),
        };
        let json = serde_json::to_value(&req).expect("should serialize");
        assert_eq!(json["systemPrompt"], "You are helpful.");
        assert_eq!(json["userContent"], "Hello");
        assert_eq!(json["maxTokens"], 100);
    }

    #[test]
    fn ai_completion_request_deserialization() {
        let json = r#"{"systemPrompt": "sys", "userContent": "usr", "maxTokens": 50}"#;
        let req: AiCompletionRequest = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(req.system_prompt, "sys");
        assert_eq!(req.user_content, "usr");
        assert_eq!(req.max_tokens, Some(50));
    }

    #[test]
    fn ai_completion_request_optional_max_tokens() {
        let json = r#"{"systemPrompt": "sys", "userContent": "usr"}"#;
        let req: AiCompletionRequest = serde_json::from_str(json).expect("should deserialize");
        assert!(req.max_tokens.is_none());
    }
}
