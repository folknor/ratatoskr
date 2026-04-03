use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};

use super::parse::{self, CardDavContactEntry};

/// Authentication method for the CardDAV server.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// HTTP Basic authentication (username + password).
    Basic,
    /// OAuth2 Bearer token (the password field contains the access token).
    OAuth2,
}

/// A minimal CardDAV client using raw reqwest + quick-xml.
///
/// Supports the subset of CardDAV needed for contact sync:
/// `PROPFIND`, `REPORT` (addressbook-multiget), and `GET`.
#[derive(Debug, Clone)]
pub struct CardDavClient {
    http: reqwest::Client,
    base_url: String,
    username: String,
    password: String,
    auth_method: AuthMethod,
    /// Discovered principal URL.
    principal_url: Option<String>,
    /// Discovered addressbook URL.
    addressbook_url: Option<String>,
}

/// Batch size for addressbook-multiget REPORT requests.
const MULTIGET_BATCH_SIZE: usize = 50;

impl CardDavClient {
    /// Create a new `CardDavClient` with the given credentials.
    ///
    /// Call [`discover`] after construction to auto-detect the principal and
    /// addressbook URLs, or set them manually with [`set_addressbook_url`].
    pub fn new(base_url: &str, username: &str, password: &str, auth_method: AuthMethod) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(5))
            .timeout(crate::constants::DAV_CLIENT_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
            auth_method,
            principal_url: None,
            addressbook_url: None,
        }
    }

    /// Override the addressbook URL (skip discovery).
    pub fn set_addressbook_url(&mut self, url: &str) {
        self.addressbook_url = Some(url.to_string());
    }

    /// Return the discovered (or manually set) addressbook URL.
    pub fn addressbook_url(&self) -> Option<&str> {
        self.addressbook_url.as_deref()
    }

    // -----------------------------------------------------------------------
    // Discovery
    // -----------------------------------------------------------------------

    /// Auto-discover the principal and addressbook URLs.
    ///
    /// 1. `GET {base_url}/.well-known/carddav` (follow redirects)
    /// 2. `PROPFIND` for `current-user-principal`
    /// 3. `PROPFIND` on the principal for `addressbook-home-set`
    /// 4. Store the discovered URLs
    pub async fn discover(&mut self) -> Result<(), String> {
        // Step 1: Try .well-known/carddav to find the DAV root
        let well_known_url = format!("{}/.well-known/carddav", self.base_url);
        let dav_root = match self
            .propfind_raw(&well_known_url, "0", PROPFIND_PRINCIPAL)
            .await
        {
            Ok((_, body)) => {
                // If we got a response, try to extract principal from it
                if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                    self.principal_url = Some(self.resolve_url(&principal));
                    self.resolve_url(&principal)
                } else {
                    // Use the well-known URL as DAV root
                    well_known_url.clone()
                }
            }
            Err(_) => {
                // .well-known not available, try the base URL directly
                self.base_url.clone()
            }
        };

        // Step 2: If we don't have a principal yet, PROPFIND on the DAV root
        if self.principal_url.is_none() {
            let (_, body) = self
                .propfind_raw(&dav_root, "0", PROPFIND_PRINCIPAL)
                .await
                .map_err(|e| format!("PROPFIND for principal failed: {e}"))?;

            if let Some(principal) = extract_href_property(&body, "current-user-principal") {
                self.principal_url = Some(self.resolve_url(&principal));
            } else {
                return Err("Could not discover current-user-principal".to_string());
            }
        }

        // Step 3: PROPFIND on the principal for addressbook-home-set
        let principal = self
            .principal_url
            .as_ref()
            .ok_or("No principal URL")?
            .clone();

        let (_, body) = self
            .propfind_raw(&principal, "0", PROPFIND_ADDRESSBOOK_HOME)
            .await
            .map_err(|e| format!("PROPFIND for addressbook-home-set failed: {e}"))?;

        if let Some(home) = extract_href_property(&body, "addressbook-home-set") {
            self.addressbook_url = Some(self.resolve_url(&home));
        } else {
            return Err("Could not discover addressbook-home-set".to_string());
        }

        log::info!(
            "CardDAV discovery complete: addressbook={}",
            self.addressbook_url.as_deref().unwrap_or("?")
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Contact listing
    // -----------------------------------------------------------------------

    /// List all contacts in the addressbook (URIs + ETags).
    pub async fn list_contacts(&self) -> Result<Vec<CardDavContactEntry>, String> {
        let url = self.require_addressbook_url()?;
        let (_, body) = self
            .propfind_raw(&url, "1", PROPFIND_CONTACTS)
            .await
            .map_err(|e| format!("PROPFIND contacts failed: {e}"))?;

        Ok(parse::parse_propfind_contacts(&body))
    }

    // -----------------------------------------------------------------------
    // vCard fetching
    // -----------------------------------------------------------------------

    /// Batch-fetch vCards by URI using addressbook-multiget REPORT.
    ///
    /// URIs are batched in groups of 50 to avoid overwhelming the server.
    /// Returns `Vec<(uri, vcard_data)>`.
    pub async fn fetch_vcards(&self, uris: &[&str]) -> Result<Vec<(String, String)>, String> {
        if uris.is_empty() {
            return Ok(Vec::new());
        }

        let addressbook_url = self.require_addressbook_url()?;
        let mut all_results = Vec::new();

        for chunk in uris.chunks(MULTIGET_BATCH_SIZE) {
            let mut href_elements = String::new();
            for uri in chunk {
                href_elements.push_str(&format!("  <D:href>{uri}</D:href>\n"));
            }

            let body = format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<C:addressbook-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
  <D:prop>
    <D:getetag/>
    <C:address-data/>
  </D:prop>
{href_elements}</C:addressbook-multiget>"#
            );

            let (_, response_body) = self
                .report_raw(&addressbook_url, &body)
                .await
                .map_err(|e| format!("REPORT multiget failed: {e}"))?;

            let parsed = parse::parse_multiget_report(&response_body);
            all_results.extend(parsed);
        }

        Ok(all_results)
    }

    // -----------------------------------------------------------------------
    // CTag
    // -----------------------------------------------------------------------

    /// Get the collection CTag for change detection.
    pub async fn get_ctag(&self) -> Result<Option<String>, String> {
        let url = self.require_addressbook_url()?;
        let (_, body) = self
            .propfind_raw(&url, "0", PROPFIND_CTAG)
            .await
            .map_err(|e| format!("PROPFIND ctag failed: {e}"))?;

        Ok(parse::parse_ctag(&body))
    }

    // -----------------------------------------------------------------------
    // HTTP helpers
    // -----------------------------------------------------------------------

    /// Send a PROPFIND request and return `(status, body)`.
    async fn propfind_raw(
        &self,
        url: &str,
        depth: &str,
        body: &str,
    ) -> Result<(StatusCode, String), String> {
        let resp = self
            .http
            .request(
                Method::from_bytes(b"PROPFIND").map_err(|e| format!("method: {e}"))?,
                url,
            )
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", depth)
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("PROPFIND {url}: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

        if status.is_success() || status == StatusCode::MULTI_STATUS {
            Ok((status, text))
        } else {
            Err(format!("PROPFIND {url} returned {status}: {text}"))
        }
    }

    /// Send a REPORT request and return `(status, body)`.
    async fn report_raw(&self, url: &str, body: &str) -> Result<(StatusCode, String), String> {
        let resp = self
            .http
            .request(
                Method::from_bytes(b"REPORT").map_err(|e| format!("method: {e}"))?,
                url,
            )
            .header(CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", "1")
            .headers(self.auth_headers())
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("REPORT {url}: {e}"))?;

        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;

        if status.is_success() || status == StatusCode::MULTI_STATUS {
            Ok((status, text))
        } else {
            Err(format!("REPORT {url} returned {status}: {text}"))
        }
    }

    /// Build authentication headers based on the auth method.
    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        match self.auth_method {
            AuthMethod::Basic => {
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{}:{}", self.username, self.password),
                );
                if let Ok(val) = format!("Basic {credentials}").parse() {
                    headers.insert(AUTHORIZATION, val);
                }
            }
            AuthMethod::OAuth2 => {
                if let Ok(val) = format!("Bearer {}", self.password).parse() {
                    headers.insert(AUTHORIZATION, val);
                }
            }
        }
        headers
    }

    /// Resolve a possibly-relative URL against the base URL.
    fn resolve_url(&self, href: &str) -> String {
        if href.starts_with("http://") || href.starts_with("https://") {
            return href.to_string();
        }
        // Extract scheme + host from base_url
        if let Ok(base) = url::Url::parse(&self.base_url)
            && let Ok(resolved) = base.join(href)
        {
            return resolved.to_string();
        }
        // Fallback: just concatenate
        format!("{}{href}", self.base_url)
    }

    /// Get the addressbook URL or return an error.
    fn require_addressbook_url(&self) -> Result<String, String> {
        self.addressbook_url.clone().ok_or_else(|| {
            "No addressbook URL — call discover() or set_addressbook_url() first".to_string()
        })
    }
}

// ---------------------------------------------------------------------------
// XML request bodies
// ---------------------------------------------------------------------------

const PROPFIND_PRINCIPAL: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:current-user-principal/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_ADDRESSBOOK_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
  <D:prop>
    <C:addressbook-home-set/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_CONTACTS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:getetag/>
    <D:getcontenttype/>
  </D:prop>
</D:propfind>"#;

const PROPFIND_CTAG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:" xmlns:CS="http://calendarserver.org/ns/">
  <D:prop>
    <CS:getctag/>
  </D:prop>
</D:propfind>"#;

// ---------------------------------------------------------------------------
// XML response extraction helpers
// ---------------------------------------------------------------------------

/// Extract an `<D:href>` value nested inside a named property element.
///
/// For example, `current-user-principal` contains `<D:href>/principals/user/</D:href>`.
fn extract_href_property(xml: &str, property_name: &str) -> Option<String> {
    use quick_xml::Reader;
    use quick_xml::escape::unescape;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);
    let mut in_property = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = local_name(e.name().as_ref());
                if name == property_name {
                    in_property = true;
                }
                current_tag = name;
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(raw) = std::str::from_utf8(e.as_ref())
                    && let Ok(text) = unescape(raw)
                {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = local_name(e.name().as_ref());
                if in_property && current_tag == "href" {
                    let val = buf.trim().to_string();
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
                if name == property_name {
                    in_property = false;
                }
                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    None
}

/// Extract the local name from a possibly-namespaced XML tag.
fn local_name(raw: &[u8]) -> String {
    let full = String::from_utf8_lossy(raw);
    match full.rfind(':') {
        Some(idx) => full[idx + 1..].to_string(),
        None => full.to_string(),
    }
}
