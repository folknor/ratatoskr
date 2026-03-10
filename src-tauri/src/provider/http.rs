/// Retry configuration for HTTP requests.
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 1000,
        }
    }
}

/// Compute retry delay from a `Retry-After` header (seconds) or exponential
/// backoff. Respects the server's `Retry-After` value when present.
pub fn compute_retry_delay(
    response: Option<&reqwest::Response>,
    attempt: u32,
    config: &RetryConfig,
) -> u64 {
    if let Some(resp) = response
        && let Some(retry_after) = resp.headers().get("Retry-After")
        && let Ok(s) = retry_after.to_str()
        && let Ok(secs) = s.parse::<u64>()
    {
        return secs * 1000;
    }
    config.initial_backoff_ms * 2u64.pow(attempt)
}
