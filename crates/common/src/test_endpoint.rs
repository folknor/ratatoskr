pub fn endpoint_has_non_root_path(endpoint: &str) -> bool {
    let after_authority = endpoint
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(endpoint);
    after_authority
        .find('/')
        .map(|idx| !after_authority[idx..].trim_matches('/').is_empty())
        .unwrap_or(false)
}

pub fn api_base_from_test_endpoint(endpoint: &str, root_path: &str) -> Option<String> {
    let endpoint = endpoint.trim().trim_end_matches('/');
    if endpoint.is_empty() {
        return None;
    }
    if endpoint_has_non_root_path(endpoint) {
        Some(endpoint.to_string())
    } else {
        Some(format!("{endpoint}/{}", root_path.trim_matches('/')))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_maps_to_root_path() {
        let base = api_base_from_test_endpoint("http://127.0.0.1:8080", "calendar/v3")
            .expect("endpoint maps");
        assert_eq!(base, "http://127.0.0.1:8080/calendar/v3");
    }

    #[test]
    fn explicit_path_is_preserved() {
        let base = api_base_from_test_endpoint("http://127.0.0.1:8080/custom", "v1")
            .expect("endpoint maps");
        assert_eq!(base, "http://127.0.0.1:8080/custom");
    }

    #[test]
    fn blank_endpoint_is_ignored() {
        assert_eq!(api_base_from_test_endpoint("   ", "v1"), None);
    }
}
