use rtsk::provider::http::RetryConfig;
pub(crate) use rtsk::provider::http::shared_http_client;

pub mod actions;
pub mod caldav;
pub mod google;
pub mod graph;
pub mod jmap;
pub mod sync;
pub mod types;

pub(crate) const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
pub(crate) const GOOGLE_CALENDAR_RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

pub(crate) fn google_calendar_api_base() -> String {
    if let Ok(value) = std::env::var("RATATOSKR_TEST_GCAL_ENDPOINT")
        && let Some(api_base) =
            rtsk::provider::test_endpoint::api_base_from_test_endpoint(
                &value,
                "calendar/v3",
            )
    {
        return api_base;
    }

    GOOGLE_CALENDAR_API_BASE.to_string()
}
