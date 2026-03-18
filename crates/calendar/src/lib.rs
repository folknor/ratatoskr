use ratatoskr_core::provider::http::RetryConfig;

pub mod caldav;
pub mod google;
pub mod sync;
pub mod types;

pub const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
pub const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
pub const GOOGLE_CALENDAR_RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

pub fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}
