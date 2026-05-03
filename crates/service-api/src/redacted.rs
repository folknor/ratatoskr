use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RedactedString(String);

impl RedactedString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for RedactedString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted len={}>", self.0.len())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RedactedBytes(Vec<u8>);

impl RedactedBytes {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for RedactedBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted len={}>", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Debug impl must never reveal the inner string content. Anything
    /// reaching `<app_data>/logs/service.<pid>.log` via a stray `{:?}` print
    /// is a sensitive-value policy violation; pin the contract here.
    #[test]
    fn redacted_string_debug_does_not_reveal_content() {
        let secret = "OAUTH-BEARER-TOKEN-SHOULD-NOT-LEAK";
        let value = RedactedString::new(secret);
        let formatted = format!("{value:?}");
        assert!(
            formatted.starts_with("<redacted"),
            "expected <redacted ...> prefix, got {formatted:?}",
        );
        assert!(
            formatted.contains(&format!("len={}", secret.len())),
            "expected length annotation, got {formatted:?}",
        );
        assert!(
            !formatted.contains(secret),
            "Debug output leaked secret: {formatted:?}",
        );
    }

    #[test]
    fn redacted_string_empty_still_redacts() {
        let value = RedactedString::new("");
        let formatted = format!("{value:?}");
        assert_eq!(formatted, "<redacted len=0>");
    }

    #[test]
    fn redacted_bytes_debug_does_not_reveal_content() {
        let secret: Vec<u8> = b"BEARER-TOKEN-PAYLOAD".to_vec();
        let len = secret.len();
        let value = RedactedBytes::new(secret.clone());
        let formatted = format!("{value:?}");
        assert!(
            formatted.starts_with("<redacted"),
            "expected <redacted ...> prefix, got {formatted:?}",
        );
        assert!(
            formatted.contains(&format!("len={len}")),
            "expected length annotation, got {formatted:?}",
        );
        let secret_utf8 = std::str::from_utf8(&secret).expect("ascii fixture");
        assert!(
            !formatted.contains(secret_utf8),
            "Debug output leaked bytes: {formatted:?}",
        );
    }

    #[test]
    fn redacted_bytes_empty_still_redacts() {
        let value = RedactedBytes::new(Vec::new());
        let formatted = format!("{value:?}");
        assert_eq!(formatted, "<redacted len=0>");
    }
}
