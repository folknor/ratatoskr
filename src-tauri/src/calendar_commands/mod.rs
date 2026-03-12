use crate::provider::http::RetryConfig;

pub(crate) mod caldav;
pub(crate) mod google;
pub(crate) mod sync;
pub(crate) mod types;

pub(super) const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
pub(super) const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
pub(super) const GOOGLE_CALENDAR_RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

pub(super) fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}
