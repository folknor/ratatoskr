use super::ParsedQuery;
use super::dates::parse_date_to_timestamp;
use types::DateBound;

/// MIME type expansions for `has:` operator values.
const HAS_EXPANSIONS: &[(&str, &[&str])] = &[
    ("pdf", &["application/pdf"]),
    (
        "image",
        &[
            "image/jpeg",
            "image/png",
            "image/gif",
            "image/webp",
            "image/svg+xml",
        ],
    ),
    (
        "excel",
        &[
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.oasis.opendocument.spreadsheet",
            "text/csv",
        ],
    ),
    (
        "word",
        &[
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.oasis.opendocument.text",
            "application/rtf",
        ],
    ),
    (
        "powerpoint",
        &[
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "application/vnd.oasis.opendocument.presentation",
        ],
    ),
    (
        "archive",
        &[
            "application/zip",
            "application/gzip",
            "application/x-tar",
            "application/x-7z-compressed",
            "application/x-rar-compressed",
        ],
    ),
    ("video", &["video/*"]),
    ("audio", &["audio/*"]),
    ("calendar", &["text/calendar", "application/ics"]),
];

// -- Operator application --

/// Apply a parsed operator to the query result.
///
/// Returns `true` if the operator+value was recognized, `false` if the value
/// was unknown (so the span should be kept as free text).
pub(super) fn apply_operator(result: &mut ParsedQuery, operator: &str, value: &str) -> bool {
    match operator {
        "from" => result.from.push(value.to_owned()),
        "to" => result.to.push(value.to_owned()),
        "has" => return apply_has_operator(result, value),
        "is" => return apply_is_operator(result, value),
        "before" => {
            result.before = parse_date_to_timestamp(value).map(DateBound::before);
        }
        "after" => {
            result.after = parse_date_to_timestamp(value).map(DateBound::after);
        }
        "label" => result.label.push(value.to_owned()),
        "account" => result.account.push(value.to_owned()),
        "folder" => result.folder.push(value.to_owned()),
        "in" => result.in_folder.push(value.to_owned()),
        "type" => result.attachment_types.push(value.to_owned()),
        _ => return false,
    }
    true
}

fn apply_has_operator(result: &mut ParsedQuery, value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "attachment" => {
            result.has_attachment = true;
            true
        }
        "contact" => {
            result.has_contact = true;
            true
        }
        _ => expand_has_value(result, &lower),
    }
}

/// Expand a `has:` value into MIME types via the expansion table.
///
/// Returns `true` if the value was recognized, `false` otherwise.
fn expand_has_value(result: &mut ParsedQuery, value: &str) -> bool {
    // Handle aliases first.
    match value {
        "spreadsheet" => {
            push_expansion_mimes(result, "excel");
            return true;
        }
        "document" => {
            push_expansion_mimes(result, "word");
            push_expansion_mimes(result, "pdf");
            return true;
        }
        _ => {}
    }

    push_expansion_mimes(result, value)
}

/// Push MIME types from the expansion table for a given key.
///
/// Returns `true` if the key was found in the expansion table.
fn push_expansion_mimes(result: &mut ParsedQuery, key: &str) -> bool {
    for &(name, mimes) in HAS_EXPANSIONS {
        if name == key {
            for &mime in mimes {
                result.attachment_types.push(mime.to_owned());
            }
            return true;
        }
    }
    false
}

fn apply_is_operator(result: &mut ParsedQuery, value: &str) -> bool {
    match value.to_ascii_lowercase().as_str() {
        "unread" => result.is_unread = Some(true),
        "read" => result.is_read = Some(true),
        "starred" => result.is_starred = Some(true),
        "snoozed" => result.is_snoozed = Some(true),
        "pinned" => result.is_pinned = Some(true),
        "muted" => result.is_muted = Some(true),
        "tagged" => result.is_tagged = Some(true),
        _ => return false,
    }
    true
}
