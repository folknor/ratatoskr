use serde::{Deserialize, Serialize};

/// A single address observation extracted from a message.
#[derive(Debug, Clone)]
pub struct AddressObservation {
    /// Lowercase-canonicalized email address.
    pub email: String,
    /// Display name if available.
    pub display_name: Option<String>,
    /// How this address relates to the user in this message.
    pub direction: Direction,
    /// Message date in unix epoch milliseconds.
    pub date_ms: i64,
}

/// The relationship direction of an observed address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// User sent directly to this address.
    SentTo,
    /// User CC'd this address.
    SentCc,
    /// User received a message from this address.
    ReceivedFrom,
    /// This address appeared in CC on a received message.
    ReceivedCc,
}

/// Result from autocomplete search across seen_addresses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeenAddressMatch {
    pub email: String,
    pub display_name: Option<String>,
    pub score: f64,
    pub account_id: String,
}

/// Parameters for extracting observations from a single message.
pub struct ObservationParams<'a> {
    /// All email addresses belonging to the user (primary + send-as aliases).
    pub self_emails: &'a [String],
    pub from_address: Option<&'a str>,
    pub from_name: Option<&'a str>,
    pub to_addresses: Option<&'a str>,
    pub cc_addresses: Option<&'a str>,
    pub bcc_addresses: Option<&'a str>,
    pub date_ms: i64,
}
