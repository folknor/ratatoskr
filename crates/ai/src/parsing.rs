use std::sync::LazyLock;

use regex::Regex;
use serde::Deserialize;

use crate::types::{ExtractedTask, TaskPriority};
use ratatoskr_core::categorization::ThreadCategory;

// ---------------------------------------------------------------------------
// Compiled regexes
// ---------------------------------------------------------------------------

/// Matches a JSON array: `[...]`
static RE_JSON_ARRAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[[\s\S]*?\]").expect("valid regex"));

/// Matches a JSON object: `{...}` (greedy — outermost braces)
static RE_JSON_OBJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[\s\S]*\}").expect("valid regex"));

/// Matches numbered list prefixes like `1. `, `2) `, etc.
static RE_NUMBERED_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d+[.)]\s*").expect("valid regex"));

/// Matches HTML tags for stripping.
static RE_HTML_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]*>").expect("valid regex"));

// ---------------------------------------------------------------------------
// Smart reply parsing
// ---------------------------------------------------------------------------

const SMART_REPLY_LIMIT: usize = 200;
const SMART_REPLY_COUNT: usize = 3;
const DEFAULT_REPLY: &str = "Thanks for the update.";

/// Parse AI-generated smart replies.
///
/// Tries JSON array first, then falls back to splitting by newlines and
/// stripping numbered prefixes. Always returns exactly 3 replies, padding
/// with a default if the AI returned fewer.
pub fn parse_smart_replies(response: &str) -> Vec<String> {
    let mut replies = try_parse_json_array(response)
        .unwrap_or_else(|| parse_newline_replies(response));

    // Sanitize: strip HTML, limit length
    replies = replies
        .into_iter()
        .filter(|r| !r.is_empty())
        .map(|r| sanitize_reply(&r))
        .collect();

    // Ensure exactly 3
    while replies.len() < SMART_REPLY_COUNT {
        replies.push(DEFAULT_REPLY.to_string());
    }
    replies.truncate(SMART_REPLY_COUNT);
    replies
}

fn try_parse_json_array(response: &str) -> Option<Vec<String>> {
    if let Some(m) = RE_JSON_ARRAY.find(response)
        && let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(m.as_str())
    {
        let strings: Vec<String> = parsed
            .into_iter()
            .filter_map(|v| match v {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .collect();
        if !strings.is_empty() {
            return Some(strings);
        }
    }
    None
}

fn parse_newline_replies(response: &str) -> Vec<String> {
    response
        .lines()
        .map(|l| RE_NUMBERED_PREFIX.replace(l.trim(), "").to_string())
        .filter(|l| !l.is_empty())
        .take(SMART_REPLY_COUNT)
        .collect()
}

fn sanitize_reply(reply: &str) -> String {
    let stripped = RE_HTML_TAGS.replace_all(reply, "");
    let trimmed = stripped.trim();
    if trimmed.len() <= SMART_REPLY_LIMIT {
        trimmed.to_string()
    } else {
        // Truncate at char boundary
        let mut end = SMART_REPLY_LIMIT;
        while !trimmed.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        trimmed[..end].to_string()
    }
}

// ---------------------------------------------------------------------------
// Task extraction parsing
// ---------------------------------------------------------------------------

/// Intermediate deserialization target for the AI JSON output.
#[derive(Deserialize)]
struct RawExtractedTask {
    title: Option<String>,
    description: Option<String>,
    #[serde(alias = "dueDate", alias = "due_date")]
    due_date: Option<i64>,
    priority: Option<String>,
}

fn parse_priority(s: &str) -> TaskPriority {
    match s {
        "none" => TaskPriority::None,
        "low" => TaskPriority::Low,
        "medium" => TaskPriority::Medium,
        "high" => TaskPriority::High,
        "urgent" => TaskPriority::Urgent,
        _ => TaskPriority::Medium,
    }
}

/// Parse an AI response into an `ExtractedTask`.
///
/// Attempts to extract JSON from the response (possibly wrapped in markdown
/// code fences). Falls back to a generic task on parse failure.
pub fn parse_extracted_task(response: &str, fallback_subject: &str) -> ExtractedTask {
    let fallback = || ExtractedTask {
        title: format!("Follow up on: {fallback_subject}"),
        description: String::new(),
        due_date: None,
        priority: TaskPriority::Medium,
    };

    let json_str = match RE_JSON_OBJECT.find(response) {
        Some(m) => m.as_str(),
        None => return fallback(),
    };

    let raw: RawExtractedTask = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return fallback(),
    };

    let title = raw
        .title
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
        .unwrap_or_else(|| format!("Follow up on: {fallback_subject}"));

    let priority = raw
        .priority
        .as_deref()
        .map(parse_priority)
        .unwrap_or(TaskPriority::Medium);

    ExtractedTask {
        title,
        description: raw.description.unwrap_or_default(),
        due_date: raw.due_date,
        priority,
    }
}

// ---------------------------------------------------------------------------
// Category parsing
// ---------------------------------------------------------------------------

/// Parse AI category response lines of `THREAD_ID:CATEGORY`.
///
/// Only lines with a valid category are included. Invalid categories and
/// empty lines are silently skipped.
pub fn parse_category_response(response: &str) -> Vec<(String, ThreadCategory)> {
    response
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let colon_idx = trimmed.find(':')?;
            let thread_id = trimmed[..colon_idx].trim();
            let category_str = trimmed[colon_idx + 1..].trim();
            if thread_id.is_empty() {
                return None;
            }
            ThreadCategory::parse(category_str)
                .map(|cat| (thread_id.to_string(), cat))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Smart label parsing
// ---------------------------------------------------------------------------

/// Parse AI smart-label response lines of `THREAD_ID:LABEL_ID_1,LABEL_ID_2`.
///
/// Returns thread IDs paired with their assigned label IDs. Lines without a
/// valid colon separator or with empty parts are skipped.
pub fn parse_smart_label_response(response: &str) -> Vec<(String, Vec<String>)> {
    response
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }
            let colon_idx = trimmed.find(':')?;
            let thread_id = trimmed[..colon_idx].trim();
            let labels_part = trimmed[colon_idx + 1..].trim();
            if thread_id.is_empty() || labels_part.is_empty() {
                return None;
            }
            let label_ids: Vec<String> = labels_part
                .split(',')
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            if label_ids.is_empty() {
                return None;
            }
            Some((thread_id.to_string(), label_ids))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Smart reply parsing --

    #[test]
    fn parse_smart_replies_valid_json() {
        let input = r#"["Got it, thanks!", "Will do.", "Sounds good to me."]"#;
        let replies = parse_smart_replies(input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], "Got it, thanks!");
        assert_eq!(replies[1], "Will do.");
        assert_eq!(replies[2], "Sounds good to me.");
    }

    #[test]
    fn parse_smart_replies_markdown_wrapped() {
        let input = "```json\n[\"Reply one\", \"Reply two\", \"Reply three\"]\n```";
        let replies = parse_smart_replies(input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], "Reply one");
    }

    #[test]
    fn parse_smart_replies_numbered_lines() {
        let input = "1. Sure thing!\n2. I'll look into it.\n3. Thanks for letting me know.";
        let replies = parse_smart_replies(input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], "Sure thing!");
        assert_eq!(replies[1], "I'll look into it.");
        assert_eq!(replies[2], "Thanks for letting me know.");
    }

    #[test]
    fn parse_smart_replies_pads_to_three() {
        let input = r#"["Only one reply"]"#;
        let replies = parse_smart_replies(input);
        assert_eq!(replies.len(), 3);
        assert_eq!(replies[0], "Only one reply");
        assert_eq!(replies[1], DEFAULT_REPLY);
        assert_eq!(replies[2], DEFAULT_REPLY);
    }

    #[test]
    fn parse_smart_replies_truncates_to_three() {
        let input =
            r#"["One", "Two", "Three", "Four", "Five"]"#;
        let replies = parse_smart_replies(input);
        assert_eq!(replies.len(), 3);
    }

    #[test]
    fn parse_smart_replies_strips_html() {
        let input = r#"["<b>Bold</b> reply", "Clean reply", "Another"]"#;
        let replies = parse_smart_replies(input);
        assert_eq!(replies[0], "Bold reply");
    }

    #[test]
    fn parse_smart_replies_limits_length() {
        let long_reply = "a".repeat(300);
        let input = format!(r#"["{long_reply}", "Short", "Also short"]"#);
        let replies = parse_smart_replies(&input);
        assert_eq!(replies[0].len(), SMART_REPLY_LIMIT);
    }

    #[test]
    fn parse_smart_replies_empty_response() {
        let replies = parse_smart_replies("");
        assert_eq!(replies.len(), 3);
        assert!(replies.iter().all(|r| r == DEFAULT_REPLY));
    }

    #[test]
    fn parse_smart_replies_non_string_json_values() {
        // JSON array with non-string values should be filtered
        let input = r#"[42, "Valid reply", null]"#;
        let replies = parse_smart_replies(input);
        assert_eq!(replies[0], "Valid reply");
        assert_eq!(replies[1], DEFAULT_REPLY);
    }

    // -- Task extraction parsing --

    #[test]
    fn parse_extracted_task_valid_json() {
        let input = r#"{"title": "Send report", "description": "Monthly figures", "dueDate": 1700000000, "priority": "high"}"#;
        let task = parse_extracted_task(input, "Test subject");
        assert_eq!(task.title, "Send report");
        assert_eq!(task.description, "Monthly figures");
        assert_eq!(task.due_date, Some(1700000000));
        assert_eq!(task.priority, TaskPriority::High);
    }

    #[test]
    fn parse_extracted_task_markdown_wrapped() {
        let input = "Here is the task:\n```json\n{\"title\": \"Do thing\", \"priority\": \"low\"}\n```";
        let task = parse_extracted_task(input, "Fallback");
        assert_eq!(task.title, "Do thing");
        assert_eq!(task.priority, TaskPriority::Low);
    }

    #[test]
    fn parse_extracted_task_invalid_json_fallback() {
        let input = "This is not JSON at all";
        let task = parse_extracted_task(input, "Important email");
        assert_eq!(task.title, "Follow up on: Important email");
        assert_eq!(task.priority, TaskPriority::Medium);
        assert!(task.description.is_empty());
        assert!(task.due_date.is_none());
    }

    #[test]
    fn parse_extracted_task_invalid_priority_defaults() {
        let input = r#"{"title": "Task", "priority": "super_urgent"}"#;
        let task = parse_extracted_task(input, "Sub");
        assert_eq!(task.priority, TaskPriority::Medium);
    }

    #[test]
    fn parse_extracted_task_missing_title() {
        let input = r#"{"description": "stuff", "priority": "low"}"#;
        let task = parse_extracted_task(input, "My subject");
        assert_eq!(task.title, "Follow up on: My subject");
    }

    #[test]
    fn parse_extracted_task_empty_title() {
        let input = r#"{"title": "   ", "priority": "high"}"#;
        let task = parse_extracted_task(input, "Fallback sub");
        assert_eq!(task.title, "Follow up on: Fallback sub");
    }

    #[test]
    fn parse_extracted_task_snake_case_due_date() {
        let input = r#"{"title": "Task", "due_date": 1700000000, "priority": "low"}"#;
        let task = parse_extracted_task(input, "Sub");
        assert_eq!(task.due_date, Some(1700000000));
    }

    // -- Category parsing --

    #[test]
    fn parse_category_response_valid() {
        let input = "thread-1:Primary\nthread-2:Updates\nthread-3:Newsletters";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], ("thread-1".to_string(), ThreadCategory::Primary));
        assert_eq!(result[1], ("thread-2".to_string(), ThreadCategory::Updates));
        assert_eq!(result[2], ("thread-3".to_string(), ThreadCategory::Newsletters));
    }

    #[test]
    fn parse_category_response_invalid_category_skipped() {
        let input = "thread-1:Primary\nthread-2:InvalidCat\nthread-3:Social";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "thread-1");
        assert_eq!(result[1].0, "thread-3");
    }

    #[test]
    fn parse_category_response_empty_lines_skipped() {
        let input = "thread-1:Primary\n\n\nthread-2:Promotions\n";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn parse_category_response_no_colon_skipped() {
        let input = "thread-1 Primary\nthread-2:Updates";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "thread-2");
    }

    #[test]
    fn parse_category_response_empty_thread_id_skipped() {
        let input = ":Primary\nthread-2:Updates";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "thread-2");
    }

    #[test]
    fn parse_category_response_whitespace_handling() {
        let input = "  thread-1 : Primary  \n  thread-2 : Social  ";
        let result = parse_category_response(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("thread-1".to_string(), ThreadCategory::Primary));
        assert_eq!(result[1], ("thread-2".to_string(), ThreadCategory::Social));
    }

    // -- Smart label parsing --

    #[test]
    fn parse_smart_label_single_label() {
        let input = "thread-1:label-a";
        let result = parse_smart_label_response(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "thread-1");
        assert_eq!(result[0].1, vec!["label-a"]);
    }

    #[test]
    fn parse_smart_label_multiple_labels() {
        let input = "thread-1:label-a,label-b,label-c";
        let result = parse_smart_label_response(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, vec!["label-a", "label-b", "label-c"]);
    }

    #[test]
    fn parse_smart_label_multiple_threads() {
        let input = "thread-1:label-a\nthread-2:label-b,label-c";
        let result = parse_smart_label_response(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "thread-1");
        assert_eq!(result[1].0, "thread-2");
        assert_eq!(result[1].1, vec!["label-b", "label-c"]);
    }

    #[test]
    fn parse_smart_label_malformed_lines() {
        let input = "thread-1:label-a\nno-colon-here\n:empty-id\nthread-2:\n\nthread-3:ok";
        let result = parse_smart_label_response(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, "thread-1");
        assert_eq!(result[1].0, "thread-3");
    }

    #[test]
    fn parse_smart_label_whitespace_in_labels() {
        let input = "thread-1: label-a , label-b ";
        let result = parse_smart_label_response(input);
        assert_eq!(result[0].1, vec!["label-a", "label-b"]);
    }
}
