use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use rusqlite::OptionalExtension;
use serde::Deserialize;
use tauri::State;

use crate::db::DbState;
use crate::provider::crypto::{AppCryptoState, decrypt_value, is_encrypted};

const DEFAULT_CLAUDE_MODEL: &str = "claude-haiku-4-5-20251001";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";
const DEFAULT_GEMINI_MODEL: &str = "gemini-2.5-flash-preview-05-20";
const DEFAULT_OLLAMA_MODEL: &str = "llama3.2";
const DEFAULT_COPILOT_MODEL: &str = "openai/gpt-4o-mini";

fn shared_ai_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCompleteRequest {
    pub system_prompt: String,
    pub user_content: String,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
enum AiProviderKind {
    Claude,
    OpenAi,
    Gemini,
    Ollama,
    Copilot,
}

struct AiConfig {
    provider: AiProviderKind,
    model: String,
    api_key: Option<String>,
    server_url: Option<String>,
}

#[tauri::command]
pub async fn ai_get_provider_name(db: State<'_, DbState>) -> Result<String, String> {
    Ok(read_plain_setting(&db, "ai_provider")
        .await?
        .as_deref()
        .map(normalize_provider_name)
        .unwrap_or("claude")
        .to_string())
}

#[tauri::command]
pub async fn ai_is_available(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<bool, String> {
    ai_is_available_impl(&db, &crypto).await
}

pub(crate) async fn ai_is_available_impl(
    db: &DbState,
    crypto: &AppCryptoState,
) -> Result<bool, String> {
    let enabled = read_plain_setting(&db, "ai_enabled").await?;
    if enabled.as_deref() == Some("false") {
        return Ok(false);
    }

    let config = load_ai_config(db, crypto.encryption_key()).await?;
    Ok(match config.provider {
        AiProviderKind::Ollama => config
            .server_url
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
        _ => config
            .api_key
            .as_ref()
            .is_some_and(|value| !value.trim().is_empty()),
    })
}

#[tauri::command]
pub async fn ai_test_connection(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<bool, String> {
    let config = load_ai_config(&db, crypto.encryption_key()).await?;
    let request = AiCompleteRequest {
        system_prompt: "You are a helpful assistant.".to_string(),
        user_content: "Say hi".to_string(),
        max_tokens: Some(16),
    };
    complete_with_config(&config, &request)
        .await
        .map(|_| true)
        .or(Ok(false))
}

#[tauri::command]
pub async fn ai_complete(
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    request: AiCompleteRequest,
) -> Result<String, String> {
    complete_ai_impl(&db, &crypto, &request).await
}

pub(crate) async fn complete_ai_impl(
    db: &DbState,
    crypto: &AppCryptoState,
    request: &AiCompleteRequest,
) -> Result<String, String> {
    let config = load_ai_config(db, crypto.encryption_key()).await?;
    complete_with_config(&config, request).await
}

async fn load_ai_config(db: &DbState, encryption_key: &[u8; 32]) -> Result<AiConfig, String> {
    let settings = load_ai_settings_map(db).await?;
    let provider_name = settings
        .get("ai_provider")
        .cloned()
        .unwrap_or_else(|| "claude".to_string());
    let provider_name = normalize_provider_name(&provider_name).to_string();
    let plain_setting = |key: &str| settings.get(key).cloned();
    let secure_setting = |key: &str| {
        settings.get(key).map(|raw| {
            if is_encrypted(raw) {
                decrypt_value(encryption_key, raw).unwrap_or_else(|_| raw.clone())
            } else {
                raw.clone()
            }
        })
    };

    match provider_name.as_str() {
        "openai" => Ok(AiConfig {
            provider: AiProviderKind::OpenAi,
            model: plain_setting("openai_model")
                .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string()),
            api_key: secure_setting("openai_api_key"),
            server_url: None,
        }),
        "gemini" => Ok(AiConfig {
            provider: AiProviderKind::Gemini,
            model: plain_setting("gemini_model")
                .unwrap_or_else(|| DEFAULT_GEMINI_MODEL.to_string()),
            api_key: secure_setting("gemini_api_key"),
            server_url: None,
        }),
        "ollama" => Ok(AiConfig {
            provider: AiProviderKind::Ollama,
            model: plain_setting("ollama_model")
                .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_string()),
            api_key: None,
            server_url: Some(
                plain_setting("ollama_server_url")
                    .unwrap_or_else(|| "http://localhost:11434".to_string()),
            ),
        }),
        "copilot" => Ok(AiConfig {
            provider: AiProviderKind::Copilot,
            model: plain_setting("copilot_model")
                .unwrap_or_else(|| DEFAULT_COPILOT_MODEL.to_string()),
            api_key: secure_setting("copilot_api_key"),
            server_url: None,
        }),
        _ => Ok(AiConfig {
            provider: AiProviderKind::Claude,
            model: plain_setting("claude_model")
                .unwrap_or_else(|| DEFAULT_CLAUDE_MODEL.to_string()),
            api_key: secure_setting("claude_api_key"),
            server_url: None,
        }),
    }
}

async fn load_ai_settings_map(
    db: &DbState,
) -> Result<std::collections::HashMap<String, String>, String> {
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
        .collect::<Result<std::collections::HashMap<_, _>, _>>()
        .map_err(|e| format!("collect AI settings: {e}"))
    })
    .await
}

fn normalize_provider_name(value: &str) -> &'static str {
    match value {
        "openai" => "openai",
        "gemini" => "gemini",
        "ollama" => "ollama",
        "copilot" => "copilot",
        _ => "claude",
    }
}

async fn read_plain_setting(db: &DbState, key: &str) -> Result<Option<String>, String> {
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
    .map_err(|e| format!("Failed to read setting {key}: {e}"))
}

async fn complete_with_config(
    config: &AiConfig,
    request: &AiCompleteRequest,
) -> Result<String, String> {
    match config.provider {
        AiProviderKind::Claude => complete_claude(config, request).await,
        AiProviderKind::OpenAi => {
            complete_openai_like(
                "https://api.openai.com/v1/chat/completions",
                config,
                request,
                None,
                true,
            )
            .await
        }
        AiProviderKind::Copilot => {
            complete_openai_like(
                "https://models.github.ai/inference/chat/completions",
                config,
                request,
                Some(("X-GitHub-Api-Version", "2022-11-28")),
                true,
            )
            .await
        }
        AiProviderKind::Ollama => {
            let base = config
                .server_url
                .as_deref()
                .ok_or_else(|| "NOT_CONFIGURED: Ollama server URL not configured".to_string())?;
            let url = format!("{}/v1/chat/completions", base.trim_end_matches('/'));
            complete_openai_like(&url, config, request, None, false).await
        }
        AiProviderKind::Gemini => complete_gemini(config, request).await,
    }
}

async fn complete_openai_like(
    url: &str,
    config: &AiConfig,
    request: &AiCompleteRequest,
    extra_header: Option<(&str, &str)>,
    require_api_key: bool,
) -> Result<String, String> {
    let api_key = if require_api_key {
        Some(config.api_key.as_deref().ok_or_else(|| {
            format!(
                "NOT_CONFIGURED: {} API key not configured",
                provider_label(config.provider)
            )
        })?)
    } else {
        None
    };

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let value = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|e| format!("NETWORK_ERROR: invalid auth header: {e}"))?;
        headers.insert(AUTHORIZATION, value);
    }
    if let Some((name, value)) = extra_header {
        headers.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| format!("NETWORK_ERROR: invalid header name: {e}"))?,
            HeaderValue::from_str(value)
                .map_err(|e| format!("NETWORK_ERROR: invalid header value: {e}"))?,
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

    let response = shared_ai_http_client()
        .post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(map_reqwest_error)?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(map_http_error(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("NETWORK_ERROR: parse AI response: {e}"))?;
    Ok(json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .to_string())
}

async fn complete_claude(config: &AiConfig, request: &AiCompleteRequest) -> Result<String, String> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| "NOT_CONFIGURED: claude API key not configured".to_string())?;
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-api-key",
        HeaderValue::from_str(api_key)
            .map_err(|e| format!("NETWORK_ERROR: invalid Anthropic API key header: {e}"))?,
    );
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    let body = serde_json::json!({
        "model": config.model,
        "max_tokens": request.max_tokens.unwrap_or(1024),
        "system": request.system_prompt,
        "messages": [{ "role": "user", "content": request.user_content }],
    });

    let response = shared_ai_http_client()
        .post("https://api.anthropic.com/v1/messages")
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(map_reqwest_error)?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(map_http_error(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("NETWORK_ERROR: parse AI response: {e}"))?;
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

async fn complete_gemini(config: &AiConfig, request: &AiCompleteRequest) -> Result<String, String> {
    let api_key = config
        .api_key
        .as_deref()
        .ok_or_else(|| "NOT_CONFIGURED: gemini API key not configured".to_string())?;
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

    let response = shared_ai_http_client()
        .post(&url)
        .header(CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(map_reqwest_error)?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(map_http_error(status.as_u16(), &text));
    }

    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("NETWORK_ERROR: parse AI response: {e}"))?;
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

fn provider_label(provider: AiProviderKind) -> &'static str {
    match provider {
        AiProviderKind::Claude => "claude",
        AiProviderKind::OpenAi => "openai",
        AiProviderKind::Gemini => "gemini",
        AiProviderKind::Ollama => "ollama",
        AiProviderKind::Copilot => "copilot",
    }
}

fn map_reqwest_error(error: reqwest::Error) -> String {
    format!("NETWORK_ERROR: {error}")
}

fn map_http_error(status: u16, body: &str) -> String {
    let lower = body.to_lowercase();
    if status == 401
        || status == 403
        || lower.contains("invalid api key")
        || lower.contains("unauthorized")
        || lower.contains("authentication")
    {
        return "AUTH_ERROR: Invalid API key".to_string();
    }
    if status == 429
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || lower.contains("resource exhausted")
    {
        return "RATE_LIMITED: Rate limited — please try again shortly".to_string();
    }
    format!("NETWORK_ERROR: HTTP {status} {body}")
}
