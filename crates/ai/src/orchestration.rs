use chrono::{TimeZone, Utc};

use crate::parsing;
use crate::prompts;
use crate::types::{
    AiCompleter, AiCompletionRequest, AiError, AiMessageInput, AiSearchResult, AutoDraftMode,
    ExtractedTask, TextTransformType, ThreadCategory, ThreadForCategorization,
    WritingStyleProfile,
};
use rtsk::db::DbState;

// ---------------------------------------------------------------------------
// Internal formatting helpers
// ---------------------------------------------------------------------------

fn format_message(msg: &AiMessageInput) -> String {
    let from = match (&msg.from_name, &msg.from_address) {
        (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
        (_, Some(addr)) => addr.clone(),
        (Some(name), None) if !name.is_empty() => name.clone(),
        _ => "Unknown".to_string(),
    };
    let date = Utc
        .timestamp_opt(msg.date, 0)
        .single()
        .map(|dt| dt.format("%b %d, %Y").to_string())
        .unwrap_or_else(|| "Unknown date".to_string());
    let body = msg
        .body_text
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .or(msg.snippet.as_deref())
        .unwrap_or("")
        .trim();
    format!("<email_content>From: {from}\nDate: {date}\n\n{body}</email_content>")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

// ---------------------------------------------------------------------------
// 1. summarize_thread
// ---------------------------------------------------------------------------

/// Summarize an email thread, using a cache to avoid redundant AI calls.
pub async fn summarize_thread(
    ai: &dyn AiCompleter,
    db: &DbState,
    account_id: &str,
    thread_id: &str,
    messages: &[AiMessageInput],
) -> Result<String, AiError> {
    // Check cache
    let cached = db_get_cache(db, account_id, thread_id, "summary").await?;
    if let Some(content) = cached {
        log::info!("AI summary cache hit for thread {thread_id}");
        return Ok(content);
    }

    let subject = messages
        .first()
        .and_then(|m| m.subject.as_deref())
        .unwrap_or("No subject");
    let formatted: String = messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n---\n");
    let combined = truncate(&format!("Subject: {subject}\n\n{formatted}"), 6000);

    let summary = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::SUMMARIZE_PROMPT.to_string(),
            user_content: combined,
            max_tokens: None,
        })
        .await?;

    db_set_cache(db, account_id, thread_id, "summary", &summary).await?;
    Ok(summary)
}

// ---------------------------------------------------------------------------
// 2. generate_smart_replies
// ---------------------------------------------------------------------------

/// Generate three short reply options for a thread.
pub async fn generate_smart_replies(
    ai: &dyn AiCompleter,
    db: &DbState,
    account_id: &str,
    thread_id: &str,
    messages: &[AiMessageInput],
) -> Result<Vec<String>, AiError> {
    // Check cache
    let cached = db_get_cache(db, account_id, thread_id, "smart_replies").await?;
    if let Some(content) = cached
        && let Ok(replies) = serde_json::from_str::<Vec<String>>(&content)
    {
        log::info!("AI smart_replies cache hit for thread {thread_id}");
        return Ok(replies);
    }
    // Corrupted cache entry (if any) — fall through and regenerate

    let formatted: String = messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n---\n");
    let combined = truncate(&formatted, 4000);

    let response = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::SMART_REPLY_PROMPT.to_string(),
            user_content: combined,
            max_tokens: None,
        })
        .await?;

    let replies = parsing::parse_smart_replies(&response);

    let json =
        serde_json::to_string(&replies).map_err(|e| AiError::ParseError(e.to_string()))?;
    db_set_cache(db, account_id, thread_id, "smart_replies", &json).await?;
    Ok(replies)
}

// ---------------------------------------------------------------------------
// 3. ask_inbox
// ---------------------------------------------------------------------------

/// Answer a question about the user's inbox using pre-fetched search results.
pub async fn ask_inbox(
    ai: &dyn AiCompleter,
    question: &str,
    search_results: &[AiSearchResult],
) -> Result<String, AiError> {
    let context: String = search_results
        .iter()
        .map(|r| {
            let date = Utc
                .timestamp_opt(r.date, 0)
                .single()
                .map(|dt| dt.format("%b %d, %Y").to_string())
                .unwrap_or_else(|| "Unknown date".to_string());
            let from = match (&r.from_name, &r.from_address) {
                (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
                (_, Some(addr)) => addr.clone(),
                (Some(name), None) if !name.is_empty() => name.clone(),
                _ => "Unknown".to_string(),
            };
            let subject = r.subject.as_deref().unwrap_or("(no subject)");
            let snippet = r.snippet.as_deref().unwrap_or("");
            format!(
                "[Message ID: {}]\nFrom: {from}\nDate: {date}\nSubject: {subject}\nPreview: {snippet}",
                r.message_id
            )
        })
        .collect::<Vec<_>>()
        .join("\n---\n");

    let user_content = format!("<email_content>{context}</email_content>\n\nQuestion: {question}");

    ai.complete(&AiCompletionRequest {
        system_prompt: prompts::ASK_INBOX_PROMPT.to_string(),
        user_content,
        max_tokens: None,
    })
    .await
}

// ---------------------------------------------------------------------------
// 4. categorize_threads
// ---------------------------------------------------------------------------

/// Categorize threads into Primary/Updates/Promotions/Social/Newsletters.
pub async fn categorize_threads(
    ai: &dyn AiCompleter,
    threads: &[ThreadForCategorization],
) -> Result<Vec<(String, ThreadCategory)>, AiError> {
    let input: String = threads
        .iter()
        .map(|t| {
            format!(
                "<email_content>ID:{} | From:{} | Subject:{} | {}</email_content>",
                t.thread_id, t.from_address, t.subject, t.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let response = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::CATEGORIZE_PROMPT.to_string(),
            user_content: input,
            max_tokens: None,
        })
        .await?;

    // Use parsing module, then filter to valid thread IDs and map to our ThreadCategory
    let valid_ids: std::collections::HashSet<&str> =
        threads.iter().map(|t| t.thread_id.as_str()).collect();

    let results: Vec<(String, ThreadCategory)> = response
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let colon_idx = trimmed.find(':')?;
            let thread_id = trimmed[..colon_idx].trim();
            let category_str = trimmed[colon_idx + 1..].trim();
            if thread_id.is_empty() || !valid_ids.contains(thread_id) {
                return None;
            }
            ThreadCategory::parse(category_str).map(|cat| (thread_id.to_string(), cat))
        })
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// 5. classify_by_smart_labels
// ---------------------------------------------------------------------------

/// Classify threads against user-defined smart label rules.
pub async fn classify_by_smart_labels(
    ai: &dyn AiCompleter,
    threads: &[ThreadForCategorization],
    label_rules: &[(String, String)],
) -> Result<Vec<(String, Vec<String>)>, AiError> {
    let label_defs: String = label_rules
        .iter()
        .map(|(id, desc)| format!("LABEL_ID:{id} — {desc}"))
        .collect::<Vec<_>>()
        .join("\n");

    let thread_data: String = threads
        .iter()
        .map(|t| {
            format!(
                "<email_content>ID:{} | From:{} | Subject:{} | {}</email_content>",
                t.thread_id, t.from_address, t.subject, t.snippet
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let user_content = format!("Label definitions:\n{label_defs}\n\nThreads:\n{thread_data}");

    let valid_thread_ids: std::collections::HashSet<&str> =
        threads.iter().map(|t| t.thread_id.as_str()).collect();
    let valid_label_ids: std::collections::HashSet<&str> =
        label_rules.iter().map(|(id, _)| id.as_str()).collect();

    let response = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::SMART_LABEL_PROMPT.to_string(),
            user_content,
            max_tokens: None,
        })
        .await?;

    let parsed = parsing::parse_smart_label_response(&response);

    // Filter to valid thread IDs and label IDs
    let results: Vec<(String, Vec<String>)> = parsed
        .into_iter()
        .filter_map(|(thread_id, labels)| {
            if !valid_thread_ids.contains(thread_id.as_str()) {
                return None;
            }
            let valid: Vec<String> = labels
                .into_iter()
                .filter(|l| valid_label_ids.contains(l.as_str()))
                .collect();
            if valid.is_empty() {
                return None;
            }
            Some((thread_id, valid))
        })
        .collect();

    Ok(results)
}

// ---------------------------------------------------------------------------
// 6. extract_task
// ---------------------------------------------------------------------------

/// Extract an actionable task from thread messages.
pub async fn extract_task(
    ai: &dyn AiCompleter,
    messages: &[AiMessageInput],
    thread_subject: &str,
) -> Result<ExtractedTask, AiError> {
    let formatted: String = messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n---\n");
    let combined = truncate(
        &format!("<email_content>Subject: {thread_subject}\n\n{formatted}</email_content>"),
        6000,
    );

    let response = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::EXTRACT_TASK_PROMPT.to_string(),
            user_content: combined,
            max_tokens: None,
        })
        .await?;

    Ok(parsing::parse_extracted_task(&response, thread_subject))
}

// ---------------------------------------------------------------------------
// 7. transform_text
// ---------------------------------------------------------------------------

/// Transform text according to the given transform type.
pub async fn transform_text(
    ai: &dyn AiCompleter,
    text: &str,
    transform_type: TextTransformType,
) -> Result<String, AiError> {
    let system_prompt = match transform_type {
        TextTransformType::Improve => prompts::IMPROVE_PROMPT,
        TextTransformType::Shorten => prompts::SHORTEN_PROMPT,
        TextTransformType::Formalize => prompts::FORMALIZE_PROMPT,
    };

    ai.complete(&AiCompletionRequest {
        system_prompt: system_prompt.to_string(),
        user_content: text.to_string(),
        max_tokens: None,
    })
    .await
}

// ---------------------------------------------------------------------------
// 8. compose_from_prompt
// ---------------------------------------------------------------------------

/// Compose an email from free-form instructions.
pub async fn compose_from_prompt(
    ai: &dyn AiCompleter,
    instructions: &str,
) -> Result<String, AiError> {
    ai.complete(&AiCompletionRequest {
        system_prompt: prompts::COMPOSE_PROMPT.to_string(),
        user_content: instructions.to_string(),
        max_tokens: None,
    })
    .await
}

// ---------------------------------------------------------------------------
// 9. generate_reply
// ---------------------------------------------------------------------------

/// Generate a reply to a thread, optionally guided by instructions.
pub async fn generate_reply(
    ai: &dyn AiCompleter,
    messages: &[AiMessageInput],
    instructions: Option<&str>,
) -> Result<String, AiError> {
    let formatted: String = messages
        .iter()
        .map(format_message)
        .collect::<Vec<_>>()
        .join("\n---\n");
    let combined = truncate(&formatted, 4000);

    let user_content = match instructions {
        Some(instr) => {
            format!("<email_content>{combined}</email_content>\n\nInstructions: {instr}")
        }
        None => format!("<email_content>{combined}</email_content>"),
    };

    ai.complete(&AiCompletionRequest {
        system_prompt: prompts::REPLY_PROMPT.to_string(),
        user_content,
        max_tokens: None,
    })
    .await
}

// ---------------------------------------------------------------------------
// 10. analyze_writing_style
// ---------------------------------------------------------------------------

/// Analyze writing style from sent email samples and store the profile.
pub async fn analyze_writing_style(
    ai: &dyn AiCompleter,
    db: &DbState,
    account_id: &str,
    sent_messages: &[AiMessageInput],
) -> Result<WritingStyleProfile, AiError> {
    let formatted: String = sent_messages
        .iter()
        .map(|msg| {
            let body = msg
                .body_text
                .as_deref()
                .or(msg.snippet.as_deref())
                .unwrap_or("")
                .trim();
            let truncated = truncate(body, 1000);
            format!("--- Sample ---\n{truncated}")
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let input = truncate(&formatted, 8000);

    let profile_text = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::WRITING_STYLE_ANALYSIS_PROMPT.to_string(),
            user_content: input,
            max_tokens: None,
        })
        .await?;

    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let sample_count = sent_messages.len() as i64;

    // Store in DB
    rtsk::db::queries_extra::db_upsert_writing_style_profile(
        db,
        account_id.to_string(),
        profile_text.clone(),
        sample_count,
    )
    .await
    .map_err(AiError::DbError)?;

    Ok(WritingStyleProfile {
        profile_text,
        sample_count,
    })
}

// ---------------------------------------------------------------------------
// 11. generate_auto_draft
// ---------------------------------------------------------------------------

/// Generate an auto-draft reply for a thread, caching the result.
pub async fn generate_auto_draft(
    ai: &dyn AiCompleter,
    db: &DbState,
    account_id: &str,
    thread_id: &str,
    messages: &[AiMessageInput],
    writing_style: &str,
    mode: AutoDraftMode,
) -> Result<String, AiError> {
    let cache_type = mode.cache_type();

    // Check cache
    let cached = db_get_cache(db, account_id, thread_id, cache_type).await?;
    if let Some(content) = cached {
        log::info!("AI {cache_type} cache hit for thread {thread_id}");
        return Ok(content);
    }

    let subject = messages
        .first()
        .and_then(|m| m.subject.as_deref())
        .unwrap_or("No subject");

    let thread_content: String = messages
        .iter()
        .map(|msg| {
            let from = match (&msg.from_name, &msg.from_address) {
                (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
                (_, Some(addr)) => addr.clone(),
                (Some(name), None) if !name.is_empty() => name.clone(),
                _ => "Unknown".to_string(),
            };
            let date = Utc
                .timestamp_opt(msg.date, 0)
                .single()
                .map(|dt| dt.format("%b %d, %Y").to_string())
                .unwrap_or_else(|| "Unknown date".to_string());
            let body = msg
                .body_text
                .as_deref()
                .or(msg.snippet.as_deref())
                .unwrap_or("")
                .trim();
            format!("From: {from}\nDate: {date}\n\n{body}")
        })
        .collect::<Vec<_>>()
        .join("\n---\n");

    let style_section = if writing_style.is_empty() {
        String::new()
    } else {
        format!("\n\nUser's writing style:\n{writing_style}")
    };

    let user_content = truncate(
        &format!("<email_content>Subject: {subject}\n\n{thread_content}</email_content>{style_section}"),
        6000,
    );

    let draft = ai
        .complete(&AiCompletionRequest {
            system_prompt: prompts::AUTO_DRAFT_REPLY_PROMPT.to_string(),
            user_content,
            max_tokens: None,
        })
        .await?;

    db_set_cache(db, account_id, thread_id, cache_type, &draft).await?;
    Ok(draft)
}

// ---------------------------------------------------------------------------
// DB helpers (thin wrappers that map String errors to AiError)
// ---------------------------------------------------------------------------

async fn db_get_cache(
    db: &DbState,
    account_id: &str,
    thread_id: &str,
    cache_type: &str,
) -> Result<Option<String>, AiError> {
    rtsk::db::queries_extra::db_get_ai_cache(
        db,
        account_id.to_string(),
        thread_id.to_string(),
        cache_type.to_string(),
    )
    .await
    .map_err(AiError::DbError)
}

async fn db_set_cache(
    db: &DbState,
    account_id: &str,
    thread_id: &str,
    cache_type: &str,
    content: &str,
) -> Result<(), AiError> {
    rtsk::db::queries_extra::db_set_ai_cache(
        db,
        account_id.to_string(),
        thread_id.to_string(),
        cache_type.to_string(),
        content.to_string(),
    )
    .await
    .map_err(AiError::DbError)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_transform_type_to_prompt() {
        // Verify each transform type maps to a distinct prompt
        let improve = match TextTransformType::Improve {
            TextTransformType::Improve => prompts::IMPROVE_PROMPT,
            TextTransformType::Shorten => prompts::SHORTEN_PROMPT,
            TextTransformType::Formalize => prompts::FORMALIZE_PROMPT,
        };
        assert!(improve.contains("Improve"));

        let shorten = match TextTransformType::Shorten {
            TextTransformType::Improve => prompts::IMPROVE_PROMPT,
            TextTransformType::Shorten => prompts::SHORTEN_PROMPT,
            TextTransformType::Formalize => prompts::FORMALIZE_PROMPT,
        };
        assert!(shorten.contains("concise"));

        let formalize = match TextTransformType::Formalize {
            TextTransformType::Improve => prompts::IMPROVE_PROMPT,
            TextTransformType::Shorten => prompts::SHORTEN_PROMPT,
            TextTransformType::Formalize => prompts::FORMALIZE_PROMPT,
        };
        assert!(formalize.contains("formal"));
    }

    #[test]
    fn auto_draft_mode_cache_key() {
        assert_eq!(AutoDraftMode::Reply.cache_type(), "auto_draft_reply");
        assert_eq!(AutoDraftMode::ReplyAll.cache_type(), "auto_draft_replyAll");
    }

    #[test]
    fn thread_for_categorization_formatting() {
        let threads = vec![
            ThreadForCategorization {
                thread_id: "t1".to_string(),
                subject: "Meeting notes".to_string(),
                snippet: "Here are the notes".to_string(),
                from_address: "alice@example.com".to_string(),
            },
            ThreadForCategorization {
                thread_id: "t2".to_string(),
                subject: "Sale!".to_string(),
                snippet: "50% off everything".to_string(),
                from_address: "promo@store.com".to_string(),
            },
        ];

        let formatted: String = threads
            .iter()
            .map(|t| {
                format!(
                    "<email_content>ID:{} | From:{} | Subject:{} | {}</email_content>",
                    t.thread_id, t.from_address, t.subject, t.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(formatted.contains("ID:t1"));
        assert!(formatted.contains("From:alice@example.com"));
        assert!(formatted.contains("Subject:Meeting notes"));
        assert!(formatted.contains("ID:t2"));
        assert!(formatted.contains("From:promo@store.com"));
        assert!(formatted.contains("\n")); // Two threads joined by newline
    }

    #[test]
    fn truncate_respects_char_boundary() {
        let s = "Hello, world!";
        assert_eq!(truncate(s, 100), s);
        assert_eq!(truncate(s, 5), "Hello");
    }

    #[test]
    fn truncate_handles_multibyte() {
        // Each emoji is 4 bytes
        let s = "\u{1F600}\u{1F601}\u{1F602}"; // 12 bytes
        let result = truncate(s, 5);
        // Should truncate to first emoji (4 bytes) since 5 is not a char boundary
        assert_eq!(result, "\u{1F600}");
    }

    #[test]
    fn format_message_with_name_and_address() {
        let msg = AiMessageInput {
            from_name: Some("Alice".to_string()),
            from_address: Some("alice@example.com".to_string()),
            date: 1700000000,
            body_text: Some("Hello world".to_string()),
            snippet: None,
            subject: None,
        };
        let result = format_message(&msg);
        assert!(result.contains("From: Alice <alice@example.com>"));
        assert!(result.contains("Hello world"));
        assert!(result.contains("<email_content>"));
    }

    #[test]
    fn format_message_without_name() {
        let msg = AiMessageInput {
            from_name: None,
            from_address: Some("bob@example.com".to_string()),
            date: 1700000000,
            body_text: Some("Body text".to_string()),
            snippet: None,
            subject: None,
        };
        let result = format_message(&msg);
        assert!(result.contains("From: bob@example.com"));
        assert!(!result.contains("<bob@example.com>"));
    }

    #[test]
    fn format_message_falls_back_to_snippet() {
        let msg = AiMessageInput {
            from_name: None,
            from_address: Some("test@test.com".to_string()),
            date: 1700000000,
            body_text: Some("   ".to_string()), // whitespace-only body
            snippet: Some("Snippet fallback".to_string()),
            subject: None,
        };
        let result = format_message(&msg);
        assert!(result.contains("Snippet fallback"));
    }

    #[test]
    fn thread_category_roundtrip() {
        for (s, expected) in [
            ("Primary", ThreadCategory::Primary),
            ("Updates", ThreadCategory::Updates),
            ("Promotions", ThreadCategory::Promotions),
            ("Social", ThreadCategory::Social),
            ("Newsletters", ThreadCategory::Newsletters),
        ] {
            assert_eq!(ThreadCategory::parse(s), Some(expected.clone()));
            assert_eq!(expected.as_str(), s);
        }
        assert_eq!(ThreadCategory::parse("Invalid"), None);
    }
}
