use crate::db;
use crate::ui::token_input::{self, TokenInputValue};

use super::types::AccountInfo;

/// Guess a MIME type from a file name (uses the `mime_guess` crate's
/// database of 800+ extension mappings, falling back to
/// `application/octet-stream` for unknown extensions).
pub fn mime_from_extension(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

/// Parse a comma-separated address string into a `TokenInputValue`.
pub(super) fn csv_to_token_input(csv: Option<&str>) -> TokenInputValue {
    let mut tiv = TokenInputValue::new();
    let Some(s) = csv else { return tiv };
    for addr in s.split(',') {
        let addr = addr.trim();
        if addr.is_empty() {
            continue;
        }
        let id = token_input::TokenId(tiv.next_id);
        tiv.next_id += 1;
        tiv.tokens.push(token_input::Token {
            id,
            email: addr.to_string(),
            label: addr.to_string(),
            is_group: false,
            group_id: None,
            member_count: None,
        });
    }
    tiv
}

pub(super) fn accounts_to_info(accounts: &[db::Account]) -> Vec<AccountInfo> {
    accounts
        .iter()
        .map(|a| AccountInfo {
            id: a.id.clone(),
            email: a.email.clone(),
            display_name: a.display_name.clone(),
            account_name: a.account_name.clone(),
        })
        .collect()
}

/// Build an attribution line for quoted content, e.g.
/// "On Mar 19, Alice Smith <alice@corp.com> wrote:"
pub(super) fn build_attribution(name: Option<&str>, email: Option<&str>) -> String {
    let sender = match (name, email) {
        (Some(n), Some(e)) if !n.is_empty() => format!("{n} <{e}>"),
        (_, Some(e)) => e.to_string(),
        (Some(n), None) if !n.is_empty() => n.to_string(),
        _ => "someone".to_string(),
    };
    format!("{sender} wrote:")
}
