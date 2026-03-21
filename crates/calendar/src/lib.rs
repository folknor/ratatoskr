pub(crate) use ratatoskr_core::provider::http::shared_http_client;
use ratatoskr_core::provider::http::RetryConfig;

pub mod caldav;
pub mod google;
pub mod graph;
pub mod sync;
pub mod types;

pub(crate) const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
pub(crate) const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
pub(crate) const GOOGLE_CALENDAR_RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};
