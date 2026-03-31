use regex::Regex;
use std::sync::LazyLock;
use url::Url;

/// Tracking query parameter names to strip from URLs.
const TRACKING_PARAMS: &[&str] = &[
    // UTM
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    // Facebook
    "fbclid",
    "fb_action_ids",
    "fb_action_types",
    "fb_ref",
    "fb_source",
    // Google
    "gclid",
    "gclsrc",
    "dclid",
    // Mailchimp
    "mc_cid",
    "mc_eid",
    // HubSpot
    "_hsenc",
    "_hsmi",
    "__hstc",
    "__hsfp",
    "__hssc",
    // Marketo
    "mkt_tok",
    // Drip
    "__s",
    // Vero
    "vero_id",
    "vero_conv",
    // Microsoft
    "msclkid",
];

/// Returns true if the given query parameter name is a known tracking parameter.
fn is_tracking_param(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    TRACKING_PARAMS.iter().any(|&p| p == lower)
}

/// Strip known tracking query parameters from a URL string.
///
/// Properly parses the URL, removes tracking parameters, and reconstructs it.
/// Returns the original string unchanged if parsing fails or there are no query
/// parameters. Preserves fragments.
pub fn strip_tracking_params(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return url.to_owned();
    };

    // If no query string, nothing to do
    if parsed.query().is_none() {
        return url.to_owned();
    }

    let retained: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(name, _)| !is_tracking_param(name))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    if retained.is_empty() {
        parsed.set_query(None);
    } else {
        // Reconstruct query string
        let qs: Vec<String> = retained
            .iter()
            .map(|(k, v)| {
                if v.is_empty() {
                    urlencoding::encode(k).into_owned()
                } else {
                    format!("{}={}", urlencoding::encode(k), urlencoding::encode(v))
                }
            })
            .collect();
        parsed.set_query(Some(&qs.join("&")));
    }

    parsed.to_string()
}

/// Check whether a URL contains any known tracking query parameters.
///
/// Returns `true` if the URL has at least one tracking parameter that
/// would be stripped by [`strip_tracking_params`]. Useful for annotating
/// links in the UI with a tracking indicator.
pub fn has_tracking_params(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    parsed
        .query_pairs()
        .any(|(name, _)| is_tracking_param(&name))
}

static HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match href="..." or href='...' (case-insensitive on href).
    // Uses alternation instead of backreference (\2) since the regex crate
    // does not support backreferences.
    Regex::new(r#"(?i)(href\s*=\s*)(?:"([^"]*)"|'([^']*)')"#).expect("href regex should compile")
});

/// Strip tracking query parameters from all `href` attribute URLs in an HTML string.
///
/// Finds all `href="..."` (and `href='...'`) attributes and applies
/// [`strip_tracking_params`] to each URL value.
pub fn strip_tracking_params_from_html(html: &str) -> String {
    HREF_RE
        .replace_all(html, |caps: &regex::Captures<'_>| {
            let prefix = &caps[1]; // href=
            // Group 2 matched double-quoted URL, group 3 matched single-quoted URL
            let (quote, url) = if let Some(m) = caps.get(2) {
                ("\"", m.as_str())
            } else if let Some(m) = caps.get(3) {
                ("'", m.as_str())
            } else {
                return caps[0].to_string();
            };
            let cleaned = strip_tracking_params(url);
            format!("{prefix}{quote}{cleaned}{quote}")
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_query_string_unchanged() {
        let url = "https://example.com/path";
        assert_eq!(strip_tracking_params(url), url);
    }

    #[test]
    fn strips_utm_params() {
        let url = "https://example.com/page?utm_source=newsletter&utm_medium=email&id=42";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/page?id=42");
    }

    #[test]
    fn strips_all_params_removes_question_mark() {
        let url = "https://example.com/page?utm_source=newsletter&fbclid=abc123";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/page");
    }

    #[test]
    fn preserves_fragment() {
        let url = "https://example.com/page?utm_source=x&keep=1#section";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/page?keep=1#section");
    }

    #[test]
    fn strips_facebook_params() {
        let url = "https://example.com/?fbclid=abc&fb_action_ids=123&real=yes";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/?real=yes");
    }

    #[test]
    fn strips_hubspot_params() {
        let url = "https://example.com/?_hsenc=abc&_hsmi=def&__hstc=ghi&page=1";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/?page=1");
    }

    #[test]
    fn strips_microsoft_params() {
        let url = "https://example.com/?msclkid=abc&product=widget";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/?product=widget");
    }

    #[test]
    fn invalid_url_returned_unchanged() {
        let url = "not a url at all";
        assert_eq!(strip_tracking_params(url), url);
    }

    #[test]
    fn relative_url_returned_unchanged() {
        let url = "/path?utm_source=test";
        assert_eq!(strip_tracking_params(url), url);
    }

    #[test]
    fn html_href_stripping() {
        let html = r#"<a href="https://example.com/page?utm_source=email&id=5">Click</a>"#;
        let result = strip_tracking_params_from_html(html);
        assert_eq!(
            result,
            r#"<a href="https://example.com/page?id=5">Click</a>"#
        );
    }

    #[test]
    fn html_multiple_hrefs() {
        let html = r#"<a href="https://a.com/?fbclid=x">A</a> <a href="https://b.com/?ok=1">B</a>"#;
        let result = strip_tracking_params_from_html(html);
        assert_eq!(
            result,
            r#"<a href="https://a.com/">A</a> <a href="https://b.com/?ok=1">B</a>"#
        );
    }

    #[test]
    fn html_single_quote_href() {
        let html = "<a href='https://example.com/?utm_campaign=test'>Link</a>";
        let result = strip_tracking_params_from_html(html);
        assert_eq!(result, "<a href='https://example.com/'>Link</a>");
    }

    #[test]
    fn html_no_tracking_params_unchanged() {
        let html = r#"<a href="https://example.com/page?id=5">Click</a>"#;
        let result = strip_tracking_params_from_html(html);
        assert_eq!(result, html);
    }

    #[test]
    fn fragment_only_after_stripping() {
        let url = "https://example.com/page?utm_source=x#top";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/page#top");
    }

    #[test]
    fn case_insensitive_param_matching() {
        let url = "https://example.com/?UTM_SOURCE=test&id=1";
        let result = strip_tracking_params(url);
        assert_eq!(result, "https://example.com/?id=1");
    }
}
