use std::collections::HashMap;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use rusqlite::OptionalExtension;

use ratatoskr_core::db::DbState;
use ratatoskr_core::provider::crypto::{AppCryptoState, decrypt_value, is_encrypted};

use crate::types::{AiCompletionRequest, AiConfig, AiError, AiProvider};

// ---------------------------------------------------------------------------
// Shared HTTP client
// ---------------------------------------------------------------------------

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// Config loading from DB
// ---------------------------------------------------------------------------

/// Load the full AI settings map from the `settings` table.
pub async fn load_ai_settings_map(
    db: &DbState,
) -> Result<HashMap<String, String>, AiError> {
    db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT key, value
                 FROM settings
                 WHERE key IN (
                   'ai_provider',
                   'claude_model',
                   'claude_api_key',
                   'openai_model',
                   'openai_api_key',
                   'gemini_model',
                   'gemini_api_key',
                   'ollama_model',
                   'ollama_server_url',
                   'copilot_model',
                   'copilot_api_key'
                 )",
            )
            .map_err(|e| format!("prepare AI settings query: {e}"))?;
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query AI settings: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("collect AI settings: {e}"))
    })
    .await
    .map_err(AiError::DbError)
}

/// Load the resolved AI configuration (provider, model, key, URL).
pub async fn load_ai_config(
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<AiConfig, AiError> {
    let settings = load_ai_settings_map(db).await?;
    build_ai_config(&settings, encryption_key)
}

/// Build an `AiConfig` from a pre-loaded settings map.
fn build_ai_config(
    settings: &HashMap<String, String>,
    encryption_key: &[u8; 32],
) -> Result<AiConfig, AiError> {
    let provider_name = settings
        .get("ai_provider")
        .map(|s| normalize_provider_name(s))
        .unwrap_or("claude");
    let plain = |key: &str| settings.get(key).cloned();
    let secure = |key: &str| {
        settings.get(key).map(|raw| {
            if is_encrypted(raw) {
                decrypt_value(encryption_key, raw)
                    .unwrap_or_else(|_| raw.clone())
            } else {
                raw.clone()
            }
        })
    };

    let provider = AiProvider::from_str_name(provider_name)
        .unwrap_or(AiProvider::Claude);

    let (model, api_key, server_url) = match provider {
        AiProvider::OpenAi => (
            plain("openai_model"),
            secure("openai_api_key"),
            None,
        ),
        AiProvider::Gemini => (
            plain("gemini_model"),
            secure("gemini_api_key"),
            None,
        ),
        AiProvider::Ollama => (
            plain("ollama_model"),
            None,
            Some(
                plain("ollama_server_url")
                    .unwrap_or_else(|| "http://localhost:11434".to_string()),
            ),
        ),
        AiProvider::Copilot => (
            plain("copilot_model"),
            secure("copilot_api_key"),
            None,
        ),
        AiProvider::Claude => (
            plain("claude_model"),
            secure("claude_api_key"),
            None,
        ),
    };

    Ok(AiConfig {
        provider,
        model: model.unwrap_or_else(|| provider.default_model().to_string()),
        api_key,
        server_url,
    })
}

// ---------------------------------------------------------------------------
// DB helpers
// ---------------------------------------------------------------------------

/// Read a single plain-text setting value.
pub async fn read_plain_setting(
    db: &DbState,
    key: &str,
) -> Result<Option<String>, AiError> {
    let key_name = key.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            rusqlite::params![key_name],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| AiError::DbError(format!("read setting {key}: {e}")))
}

/// Check whether AI is available (enabled + configured).
pub async fn ai_is_available(
    db: &DbState,
    crypto: &AppCryptoState,
) -> Result<bool, AiError> {
    let enabled = read_plain_setting(db, "ai_enabled").await?;
    if enabled.as_deref() == Some("false") {
        return Ok(false);
    }

    let config = load_ai_config(db, crypto.encryption_key()).await?;
    Ok(match config.provider {
        AiProvider::Ollama => config
            .server_url
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty()),
        _ => config
            .api_key
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty()),
    })
}

/// Return the normalized provider name from the DB.
pub async fn get_provider_name(db: &DbState) -> Result<String, AiError> {
    Ok(read_plain_setting(db, "ai_provider")
        .await?
        .as_deref()
        .map(normalize_provider_name)
        .unwrap_or("claude")
        .to_string())
}

// ---------------------------------------------------------------------------
// Completion entry point
// ---------------------------------------------------------------------------

/// Complete an AI request using the stored config.
pub async fn complete(
    db: &DbState,
    crypto: &AppCryptoState,
    request: &AiCompletionRequest,
) -> Result<String, AiError> {
    let config = load_ai_config(db, crypto.encryption_key()).await?;
    complete_with_config(&config, request).await
}

/// Complete an AI request using an explicit config.
pub async fn complete_with_config(
    config: &AiConfig,
    request: &AiCompletionRequest,
) -> Result<String, AiError> {
    match config.provider {
        AiProvider::Claude => complete_claude(config, request).await,
        AiProvider::OpenAi => {
            complete_openai_like(
                "https://api.openai.com/v1/chat/completions",
                config,
                request,
                None,
                true,
            )
            .await
        }
        AiProvider::Copilot => {
            complete_openai_like(
                "https://models.github.ai/inference/chat/completions",
                config,
                request,
                Some(("X-GitHub-Api-Version", "2022-11-28")),
                true,
            )
            .await
        }
        AiProvider::Ollama => {
            let base = config
                .server_url
                .as_deref()
                .ok_or_else(|| {
                    AiError::NotConfigured(
                        "Ollama server URL not configured".to_string(),
                    )
                })?;
            let url = format!(
                "{}/v1/chat/completions",
                base.trim_end_matches('/')
            );
            complete_openai_like(&url, config, request, None, false).await
        }
        AiProvider::Gemini => complete_gemini(config, request).await,
    }
}

// ---------------------------------------------------------------------------
// Provider-specific HTTP calls
// ---------------------------------------------------------------------------

async fn complete_openai_like(
    url: &str,
    config: &AiConfig,
    request: &AiCompletionRequest,
    extra_header: Option<(&str, &str)>,
    require_api_key: bool,
) -> Result<String, AiError> {
    let api_key = if require_api_key {
        Some(
            config
                .api_key
                .as_deref()
                .ok_or_else(|| {
                    AiError::NotConfigured(format!(
                        "{} API key not configured",
                        config.provider
                    ))
                })?,
        )
    } else {
        config.api_key.as_deref()
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let value = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|e| {
                AiError::NetworkError(format!("invalid auth header: {e}"))
            })?;
        headers.insert(AUTHORIZATION, value);
    }
    if let Some((name, value)) = extra_header {
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| {
                    AiError::NetworkError(format!("invalid header name: {e}"))
                })?,
            HeaderValue::from_str(value).map_err(|e| {
                AiError::NetworkError(format!("invalid header value: {e}"))
            })?,
        );
    }

    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": request.max_tokens.unwrap_or(1024),
        "messages": [
            { "role": "system", "content": request.system_prompt },
            { "role": "user", "content": request.user_content }
        ]
    });

    let response = shared_http_client()
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::NetworkError(format!("{e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AiError::from_http(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AiError::NetworkError(format!("parse AI response: {e}")))?;
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

async fn complete_claude(
    config: &AiConfig,
    request: &AiCompletionRequest,
) -> Result<String, AiError> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| {
            AiError::NotConfigured(
                "claude API key not configured".to_string(),
            )
        })?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key).map_err(|e| {
            AiError::NetworkError(format!(
                "invalid Anthropic API key header: {e}"
            ))
        })?,
    );
    headers.insert(
        "anthropic-version",
        HeaderValue::from_static("2023-06-01"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": request.max_tokens.unwrap_or(1024),
        "system": request.system_prompt,
        "messages": [{ "role": "user", "content": request.user_content }],
    });

    let response = shared_http_client()
        .post("https://api.anthropic.com/v1/messages")
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::NetworkError(format!("{e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AiError::from_http(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AiError::NetworkError(format!("parse AI response: {e}")))?;
    Ok(json["content"]
        .as_array()
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|block| block["type"].as_str() == Some("text"))
                .and_then(|block| block["text"].as_str())
        })
        .unwrap_or_default()
        .to_string())
}

async fn complete_gemini(
    config: &AiConfig,
    request: &AiCompletionRequest,
) -> Result<String, AiError> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| {
            AiError::NotConfigured(
                "gemini API key not configured".to_string(),
            )
        })?;
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        urlencoding::encode(&config.model),
        urlencoding::encode(api_key)
    );
    let body = serde_json::json!({
        "systemInstruction": {
            "parts": [{ "text": request.system_prompt }]
        },
        "contents": [{
            "role": "user",
            "parts": [{ "text": request.user_content }]
        }],
        "generationConfig": {
            "maxOutputTokens": request.max_tokens.unwrap_or(1024)
        }
    });

    let response = shared_http_client()
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| AiError::NetworkError(format!("{e}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AiError::from_http(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| AiError::NetworkError(format!("parse AI response: {e}")))?;
    let result = json["candidates"][0]["content"]["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_provider_name(value: &str) -> &'static str {
    match value {
        "openai" => "openai",
        "gemini" => "gemini",
        "ollama" => "ollama",
        "copilot" => "copilot",
        _ => "claude",
    }
}

// ---------------------------------------------------------------------------
// AiCompleter implementation backed by DB config
// ---------------------------------------------------------------------------

/// An [`AiCompleter`] implementation that reads configuration from the DB
/// and calls the appropriate AI provider over HTTP.
pub struct DbConfigCompleter<'a> {
    db: &'a DbState,
    crypto: &'a AppCryptoState,
}

impl<'a> DbConfigCompleter<'a> {
    pub fn new(db: &'a DbState, crypto: &'a AppCryptoState) -> Self {
        Self { db, crypto }
    }
}

#[async_trait::async_trait]
impl crate::AiCompleter for DbConfigCompleter<'_> {
    async fn complete(
        &self,
        request: &AiCompletionRequest,
    ) -> Result<String, AiError> {
        self::complete(self.db, self.crypto, request).await
    }
}
