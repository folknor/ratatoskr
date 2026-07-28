use std::sync::LazyLock;

use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};

/// Cloud storage provider that hosts the linked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudProvider {
    OneDrive,
    GoogleDrive,
    Dropbox,
    Box,
}

impl CloudProvider {
    /// Returns the string used in the `provider` column of `cloud_attachments`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneDrive => "onedrive",
            Self::GoogleDrive => "google_drive",
            Self::Dropbox => "dropbox",
            Self::Box => "box",
        }
    }
}

/// A cloud storage link detected in an email body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudLink {
    pub url: String,
    pub provider: CloudProvider,
}

// Patterns are ordered to match CloudProvider variants via index ranges.
// Indices 0-2: OneDrive, 3-4: Google Drive, 5-6: Dropbox, 7: Box
static CLOUD_URL_PATTERNS: LazyLock<RegexSet> = LazyLock::new(|| {
    RegexSet::new([
        // OneDrive (indices 0-2)
        r"1drv\.ms",
        r"onedrive\.live\.com",
        r"sharepoint\.com",
        // Google Drive (indices 3-4)
        r"drive\.google\.com",
        r"docs\.google\.com/(?:document|spreadsheet|presentation|forms)",
        // Dropbox (indices 5-6)
        r"dropbox\.com/sh?/",
        r"dl\.dropboxusercontent\.com",
        // Box (index 7)
        r"app\.box\.com/s/",
    ])
    .expect("cloud URL patterns should be valid regexes")
});

/// Regex to extract URLs from href attributes in HTML.
static HREF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href\s*=\s*["']([^"']+)["']"#).expect("href regex should be valid")
});

fn provider_for_match_index(index: usize) -> Option<CloudProvider> {
    match index {
        0..=2 => Some(CloudProvider::OneDrive),
        3..=4 => Some(CloudProvider::GoogleDrive),
        5..=6 => Some(CloudProvider::Dropbox),
        7 => Some(CloudProvider::Box),
        _ => None,
    }
}

/// Scans an HTML body for cloud storage links and returns all detected links.
///
/// Extracts URLs from `href` attributes and matches each against known cloud
/// storage URL patterns. Each unique URL is returned at most once, tagged with
/// its provider.
pub fn detect_cloud_links(html_body: &str) -> Vec<CloudLink> {
    let mut results = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();

    for cap in HREF_RE.captures_iter(html_body) {
        let Some(url) = cap.get(1).map(|m| m.as_str()) else {
            continue;
        };

        if !seen_urls.insert(url) {
            continue;
        }

        let matches: Vec<usize> = CLOUD_URL_PATTERNS.matches(url).into_iter().collect();
        if let Some(&first_match) = matches.first()
            && let Some(provider) = provider_for_match_index(first_match)
        {
            results.push(CloudLink {
                url: url.to_owned(),
                provider,
            });
        }
    }

    results
}

/// Regex for extracting file IDs from Google Drive and Google Docs URLs.
///
/// Matches:
/// - `drive.google.com/file/d/{id}/...`
/// - `docs.google.com/document/d/{id}/...`
/// - `docs.google.com/spreadsheets/d/{id}/...`
/// - `docs.google.com/presentation/d/{id}/...`
/// - `drive.google.com/open?id={id}`
static GDRIVE_ID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:drive\.google\.com/file/d/|docs\.google\.com/(?:document|spreadsheets|presentation)/d/|drive\.google\.com/open\?id=)([a-zA-Z0-9_-]+)",
    )
    .expect("Google Drive ID regex should be valid")
});

/// Extract a Google Drive file ID from various URL formats.
///
/// Returns `None` if the URL does not match any known Google Drive pattern.
pub fn extract_gdrive_file_id(url: &str) -> Option<String> {
    GDRIVE_ID_RE
        .captures(url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_onedrive_links() {
        let html = r#"<a href="https://1drv.ms/w/s!AjJkLmNoPqRs">doc</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
    }

    #[test]
    fn detects_sharepoint_link() {
        let html = r#"<a href="https://contoso.sharepoint.com/:w:/g/personal/file">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
    }

    #[test]
    fn detects_google_drive_link() {
        let html = r#"<a href="https://drive.google.com/file/d/abc123/view">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::GoogleDrive);
    }

    #[test]
    fn detects_google_docs_link() {
        let html = r#"<a href="https://docs.google.com/spreadsheet/d/abc123/edit">sheet</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::GoogleDrive);
    }

    #[test]
    fn detects_dropbox_link() {
        let html = r#"<a href="https://www.dropbox.com/s/abc123/file.pdf?dl=0">pdf</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Dropbox);
    }

    #[test]
    fn detects_dropbox_shared_link() {
        let html = r#"<a href="https://www.dropbox.com/sh/abc123/folder">folder</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Dropbox);
    }

    #[test]
    fn detects_box_link() {
        let html = r#"<a href="https://app.box.com/s/abc123def456">file</a>"#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].provider, CloudProvider::Box);
    }

    #[test]
    fn deduplicates_urls() {
        let html = r#"
            <a href="https://drive.google.com/file/d/abc/view">link1</a>
            <a href="https://drive.google.com/file/d/abc/view">link2</a>
        "#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn ignores_non_cloud_links() {
        let html = r#"
            <a href="https://example.com/page">normal</a>
            <a href="https://github.com/repo">github</a>
        "#;
        let links = detect_cloud_links(html);
        assert!(links.is_empty());
    }

    #[test]
    fn detects_multiple_providers() {
        let html = r#"
            <a href="https://1drv.ms/w/s!AjJk">onedrive</a>
            <a href="https://drive.google.com/file/d/abc/view">gdrive</a>
            <a href="https://app.box.com/s/xyz">box</a>
        "#;
        let links = detect_cloud_links(html);
        assert_eq!(links.len(), 3);
        assert_eq!(links[0].provider, CloudProvider::OneDrive);
        assert_eq!(links[1].provider, CloudProvider::GoogleDrive);
        assert_eq!(links[2].provider, CloudProvider::Box);
    }

    #[test]
    fn extract_gdrive_file_id_from_file_url() {
        let url = "https://drive.google.com/file/d/1aBcDeFgHiJkLmNoPqRsTuVwXyZ/view?usp=sharing";
        assert_eq!(
            extract_gdrive_file_id(url),
            Some("1aBcDeFgHiJkLmNoPqRsTuVwXyZ".to_owned())
        );
    }

    #[test]
    fn extract_gdrive_file_id_from_docs_url() {
        let url = "https://docs.google.com/document/d/1aBcDeFgHiJk/edit";
        assert_eq!(extract_gdrive_file_id(url), Some("1aBcDeFgHiJk".to_owned()));
    }

    #[test]
    fn extract_gdrive_file_id_from_spreadsheets_url() {
        let url = "https://docs.google.com/spreadsheets/d/1aBcDeFgHiJk/edit#gid=0";
        assert_eq!(extract_gdrive_file_id(url), Some("1aBcDeFgHiJk".to_owned()));
    }

    #[test]
    fn extract_gdrive_file_id_from_open_url() {
        let url = "https://drive.google.com/open?id=1aBcDeFgHiJk";
        assert_eq!(extract_gdrive_file_id(url), Some("1aBcDeFgHiJk".to_owned()));
    }

    #[test]
    fn extract_gdrive_file_id_returns_none_for_non_drive_url() {
        assert_eq!(extract_gdrive_file_id("https://example.com/file"), None);
    }

    #[test]
    fn extract_gdrive_file_id_from_presentation_url() {
        let url = "https://docs.google.com/presentation/d/1aBcDeFgHiJk/edit";
        assert_eq!(extract_gdrive_file_id(url), Some("1aBcDeFgHiJk".to_owned()));
    }
}
