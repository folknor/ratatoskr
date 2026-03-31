use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures::stream::{self, StreamExt};
use hickory_resolver::TokioResolver;
use log::{debug, info, warn};
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed BIMI TXT record.
#[derive(Debug, Clone)]
struct BimiRecord {
    logo_uri: Option<String>,
    authority_uri: Option<String>,
}

/// A cached BIMI entry from the database.
#[derive(Debug, Clone)]
pub struct BimiCacheEntry {
    pub domain: String,
    pub has_bimi: bool,
    pub logo_uri: Option<String>,
    pub authority_uri: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum SVG file size (32 KiB per BIMI spec).
const MAX_SVG_SIZE: usize = 32 * 1024;

/// Cache TTL for positive BIMI results (7 days).
const POSITIVE_TTL_SECS: i64 = 7 * 24 * 3600;

/// Cache TTL for negative BIMI results (24 hours).
const NEGATIVE_TTL_SECS: i64 = 24 * 3600;

/// Rasterized icon size in pixels (square).
const ICON_SIZE: u32 = 128;

// ---------------------------------------------------------------------------
// DB cache layer (migration v38)
// ---------------------------------------------------------------------------

/// Retrieve a cached BIMI entry for a domain, returning `None` if absent or
/// expired.
pub fn get_bimi_cache(conn: &Connection, domain: &str) -> Result<Option<BimiCacheEntry>, String> {
    conn.query_row(
        "SELECT domain, has_bimi, logo_uri, authority_uri, fetched_at, expires_at \
         FROM bimi_cache WHERE domain = ?1 AND expires_at > strftime('%s', 'now')",
        rusqlite::params![domain],
        |row| {
            Ok(BimiCacheEntry {
                domain: row.get("domain")?,
                has_bimi: row.get::<_, i32>("has_bimi")? != 0,
                logo_uri: row.get("logo_uri")?,
                authority_uri: row.get("authority_uri")?,
                fetched_at: row.get("fetched_at")?,
                expires_at: row.get("expires_at")?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("bimi cache query: {e}"))
}

/// Insert or update a BIMI cache entry.
pub fn upsert_bimi_cache(
    conn: &Connection,
    domain: &str,
    has_bimi: bool,
    logo_uri: Option<&str>,
    authority_uri: Option<&str>,
    expires_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO bimi_cache (domain, has_bimi, logo_uri, authority_uri, fetched_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'), ?5) \
         ON CONFLICT(domain) DO UPDATE SET \
           has_bimi = excluded.has_bimi, \
           logo_uri = excluded.logo_uri, \
           authority_uri = excluded.authority_uri, \
           fetched_at = excluded.fetched_at, \
           expires_at = excluded.expires_at",
        rusqlite::params![domain, has_bimi as i32, logo_uri, authority_uri, expires_at],
    )
    .map_err(|e| format!("bimi cache upsert: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Authentication-Results header parsing
// ---------------------------------------------------------------------------

/// Returns `true` if the `Authentication-Results` header indicates DMARC pass.
fn dmarc_passed(authentication_results: &str) -> bool {
    authentication_results.contains("dmarc=pass")
}

// ---------------------------------------------------------------------------
// BIMI-Indicator header shortcut
// ---------------------------------------------------------------------------

/// Attempt to extract an SVG from a `BIMI-Indicator` header (base64-encoded).
/// Returns the raw SVG bytes on success.
fn decode_bimi_indicator(header_value: &str) -> Option<Vec<u8>> {
    let trimmed = header_value.trim();
    if trimmed.is_empty() {
        return None;
    }
    BASE64.decode(trimmed).ok()
}

// ---------------------------------------------------------------------------
// DNS BIMI record lookup
// ---------------------------------------------------------------------------

/// Parse a BIMI TXT record value like `v=BIMI1; l=https://...; a=https://...`
fn parse_bimi_record(txt: &str) -> Option<BimiRecord> {
    let txt = txt.trim();
    if !txt.starts_with("v=BIMI1") {
        return None;
    }

    let mut logo_uri: Option<String> = None;
    let mut authority_uri: Option<String> = None;

    for part in txt.split(';') {
        let part = part.trim();
        if let Some(val) = part.strip_prefix("l=") {
            let val = val.trim();
            if !val.is_empty() {
                logo_uri = Some(val.to_string());
            }
        } else if let Some(val) = part.strip_prefix("a=") {
            let val = val.trim();
            if !val.is_empty() {
                authority_uri = Some(val.to_string());
            }
        }
    }

    Some(BimiRecord {
        logo_uri,
        authority_uri,
    })
}

/// Extract the organizational domain by stripping the first label.
/// e.g. `mail.example.com` -> `example.com`
fn organizational_domain(domain: &str) -> Option<&str> {
    let dot = domain.find('.')?;
    let rest = &domain[dot + 1..];
    // Must still contain at least one dot to be a valid domain
    if rest.contains('.') { Some(rest) } else { None }
}

/// Look up the BIMI TXT record for a domain via DNS, falling back to the
/// organizational domain if the exact domain has no record.
async fn lookup_bimi_dns(domain: &str) -> Result<Option<BimiRecord>, String> {
    let resolver = TokioResolver::builder_tokio()
        .map_err(|e| format!("create DNS resolver: {e}"))?
        .build();

    // Try exact domain first
    let qname = format!("default._bimi.{domain}");
    debug!("BIMI DNS lookup: {qname}");

    if let Some(record) = query_bimi_txt(&resolver, &qname).await {
        return Ok(Some(record));
    }

    // Fall back to organizational domain
    if let Some(org_domain) = organizational_domain(domain) {
        let qname = format!("default._bimi.{org_domain}");
        debug!("BIMI DNS fallback lookup: {qname}");
        if let Some(record) = query_bimi_txt(&resolver, &qname).await {
            return Ok(Some(record));
        }
    }

    Ok(None)
}

/// Query DNS for BIMI TXT records at the given name. Returns `None` on any
/// error (NXDOMAIN, timeout, etc.) since BIMI is best-effort.
async fn query_bimi_txt(resolver: &TokioResolver, qname: &str) -> Option<BimiRecord> {
    match resolver.txt_lookup(qname).await {
        Ok(response) => {
            for record in response.iter() {
                let txt: String = record.to_string();
                if let Some(parsed) = parse_bimi_record(&txt) {
                    return Some(parsed);
                }
            }
            None
        }
        Err(e) => {
            debug!("BIMI DNS lookup for {qname}: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// SVG fetch and validation
// ---------------------------------------------------------------------------

/// Fetch and validate an SVG from a BIMI logo URI.
async fn fetch_and_validate_svg(logo_uri: &str) -> Result<Vec<u8>, String> {
    if !logo_uri.starts_with("https://") {
        return Err("BIMI logo URI must use HTTPS".to_string());
    }

    let response = reqwest::get(logo_uri)
        .await
        .map_err(|e| format!("fetch BIMI SVG: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("BIMI SVG fetch returned {}", response.status()));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read BIMI SVG body: {e}"))?;

    if bytes.len() > MAX_SVG_SIZE {
        return Err(format!(
            "BIMI SVG too large: {} bytes (max {MAX_SVG_SIZE})",
            bytes.len()
        ));
    }

    validate_svg(&bytes)?;

    Ok(bytes.to_vec())
}

/// Basic SVG Tiny PS compliance checks.
fn validate_svg(data: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(data).map_err(|e| format!("SVG is not valid UTF-8: {e}"))?;

    // Must declare baseProfile="tiny-ps" per BIMI spec
    if !text.contains("baseProfile=\"tiny-ps\"") && !text.contains("baseProfile='tiny-ps'") {
        return Err("BIMI SVG missing baseProfile=\"tiny-ps\"".to_string());
    }

    // Reject external URI references (xlink:href to external resources)
    // Allow data: URIs and fragment-only references
    for attr in [
        "xlink:href=\"http",
        "xlink:href='http",
        "href=\"http",
        "href='http",
    ] {
        if text.contains(attr) {
            // Allow the SVG namespace declaration itself
            let is_namespace_only = text.match_indices(attr).all(|(i, _)| {
                // Check if this is xmlns:xlink
                i >= 6 && &text[i - 6..i] == "xmlns:"
            });
            if !is_namespace_only {
                return Err("BIMI SVG contains external URI references".to_string());
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// SVG rasterization
// ---------------------------------------------------------------------------

/// Rasterize SVG data to a PNG file at the given path.
fn rasterize_svg_to_png(svg_data: &[u8], output_path: &Path) -> Result<(), String> {
    let options = resvg::usvg::Options::default();
    let tree =
        resvg::usvg::Tree::from_data(svg_data, &options).map_err(|e| format!("parse SVG: {e}"))?;

    let size = tree.size();
    let icon_f = ICON_SIZE as f32;
    let scale_x = icon_f / size.width();
    let scale_y = icon_f / size.height();
    let scale = scale_x.min(scale_y);

    let mut pixmap = resvg::tiny_skia::Pixmap::new(ICON_SIZE, ICON_SIZE)
        .ok_or_else(|| "failed to create pixmap".to_string())?;

    // Center the image
    let scaled_w = size.width() * scale;
    let scaled_h = size.height() * scale;
    let tx = (icon_f - scaled_w) / 2.0;
    let ty = (icon_f - scaled_h) / 2.0;

    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);

    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Encode as PNG
    let png_data = pixmap
        .encode_png()
        .map_err(|e| format!("encode PNG: {e}"))?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create BIMI cache dir: {e}"))?;
    }

    std::fs::write(output_path, png_data).map_err(|e| format!("write BIMI PNG: {e}"))?;

    Ok(())
}

/// Compute the filesystem cache path for a given logo URI.
fn cache_path(cache_dir: &Path, logo_uri: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(logo_uri.as_bytes());
    let hash = hex_encode(hasher.finalize());
    cache_dir.join("bimi").join(format!("{hash}.png"))
}

/// Minimal hex encoding to avoid adding a dependency.
fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Full BIMI lookup pipeline:
///
/// 1. Check DB cache for a non-expired entry
/// 2. Verify DMARC passes via `Authentication-Results` header
/// 3. Try `BIMI-Indicator` header shortcut (base64 SVG)
/// 4. DNS TXT lookup for BIMI record
/// 5. Fetch and validate SVG from logo URI
/// 6. Rasterize SVG to PNG
/// 7. Cache result in DB
/// 8. Return path to cached PNG (or `None`)
///
/// `authentication_results` and `bimi_indicator` are optional header values
/// from the message being displayed.
pub async fn lookup_bimi(
    domain: &str,
    authentication_results: Option<&str>,
    bimi_indicator: Option<&str>,
    cache_dir: &Path,
    conn: &Connection,
) -> Option<PathBuf> {
    let domain = domain.to_lowercase();

    // 1. Check cache
    match get_bimi_cache(conn, &domain) {
        Ok(Some(entry)) => {
            if !entry.has_bimi {
                debug!("BIMI cache: negative for {domain}");
                return None;
            }
            if let Some(ref uri) = entry.logo_uri {
                let path = cache_path(cache_dir, uri);
                if path.exists() {
                    debug!("BIMI cache hit for {domain}");
                    return Some(path);
                }
                // PNG was deleted; fall through to re-fetch
            }
        }
        Ok(None) => {} // no cache entry
        Err(e) => {
            warn!("BIMI cache read error: {e}");
        }
    }

    // 2. Check DMARC
    if let Some(ar) = authentication_results {
        if !dmarc_passed(ar) {
            debug!("BIMI: DMARC did not pass for {domain}");
            cache_negative(&domain, conn);
            return None;
        }
    }
    // If no Authentication-Results header is available, we still attempt the
    // lookup — some callers may have pre-verified DMARC externally.

    // 3. Try BIMI-Indicator shortcut
    if let Some(indicator) = bimi_indicator {
        if let Some(svg_data) = decode_bimi_indicator(indicator) {
            let logo_key = format!("bimi-indicator:{domain}");
            let path = cache_path(cache_dir, &logo_key);

            if !path.exists() {
                if let Err(e) = rasterize_svg_to_png(&svg_data, &path) {
                    warn!("BIMI-Indicator rasterize failed for {domain}: {e}");
                } else {
                    let now = chrono::Utc::now().timestamp();
                    let expires = now + POSITIVE_TTL_SECS;
                    if let Err(e) =
                        upsert_bimi_cache(conn, &domain, true, Some(&logo_key), None, expires)
                    {
                        warn!("failed to cache BIMI indicator for {domain}: {e}");
                    }
                    return Some(path);
                }
            } else {
                return Some(path);
            }
        }
    }

    // 4. DNS lookup
    let record = match lookup_bimi_dns(&domain).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            debug!("BIMI: no DNS record for {domain}");
            cache_negative(&domain, conn);
            return None;
        }
        Err(e) => {
            warn!("BIMI DNS lookup failed for {domain}: {e}");
            return None;
        }
    };

    let logo_uri = match record.logo_uri {
        Some(ref uri) if !uri.is_empty() => uri,
        _ => {
            debug!("BIMI: DNS record for {domain} has no logo URI");
            cache_negative(&domain, conn);
            return None;
        }
    };

    // Check if we already have the PNG cached on disk
    let path = cache_path(cache_dir, logo_uri);
    if path.exists() {
        let now = chrono::Utc::now().timestamp();
        let expires = now + POSITIVE_TTL_SECS;
        if let Err(e) = upsert_bimi_cache(
            conn,
            &domain,
            true,
            Some(logo_uri),
            record.authority_uri.as_deref(),
            expires,
        ) {
            warn!("failed to update BIMI cache for {domain}: {e}");
        }
        return Some(path);
    }

    // 5. Fetch and validate SVG
    let svg_data = match fetch_and_validate_svg(logo_uri).await {
        Ok(data) => data,
        Err(e) => {
            warn!("BIMI SVG fetch/validate failed for {domain}: {e}");
            cache_negative(&domain, conn);
            return None;
        }
    };

    // 6. Rasterize
    if let Err(e) = rasterize_svg_to_png(&svg_data, &path) {
        warn!("BIMI rasterize failed for {domain}: {e}");
        cache_negative(&domain, conn);
        return None;
    }

    // 7. Cache positive result
    let now = chrono::Utc::now().timestamp();
    let expires = now + POSITIVE_TTL_SECS;
    if let Err(e) = upsert_bimi_cache(
        conn,
        &domain,
        true,
        Some(logo_uri),
        record.authority_uri.as_deref(),
        expires,
    ) {
        warn!("failed to cache positive BIMI result for {domain}: {e}");
    }

    // 8. Return path
    Some(path)
}

/// Cache a negative (no BIMI) result.
fn cache_negative(domain: &str, conn: &Connection) {
    let now = chrono::Utc::now().timestamp();
    let expires = now + NEGATIVE_TTL_SECS;
    if let Err(e) = upsert_bimi_cache(conn, domain, false, None, None, expires) {
        warn!("failed to cache negative BIMI result for {domain}: {e}");
    }
}

// ---------------------------------------------------------------------------
// In-memory LRU cache
// ---------------------------------------------------------------------------

/// Default LRU capacity.
const LRU_CAPACITY: usize = 500;

/// In-memory LRU cache wrapping the DB/filesystem BIMI lookup.
///
/// Avoids hitting the DB and filesystem on every message render.
/// The outer `Option` in `get()` indicates cache miss (`None`) vs hit
/// (`Some(None)` = no BIMI, `Some(Some(path))` = logo path).
pub struct BimiLruCache {
    cache: Mutex<LruMap>,
}

/// Simple LRU built on a `HashMap` + insertion-order `Vec` for eviction.
struct LruMap {
    entries: HashMap<String, Option<PathBuf>>,
    order: Vec<String>,
    capacity: usize,
}

impl LruMap {
    fn new(capacity: usize) -> Self {
        Self {
            entries: HashMap::with_capacity(capacity),
            order: Vec::with_capacity(capacity),
            capacity,
        }
    }

    fn get(&mut self, key: &str) -> Option<Option<PathBuf>> {
        if self.entries.contains_key(key) {
            // Move to back (most recently used)
            self.order.retain(|k| k != key);
            self.order.push(key.to_string());
            self.entries.get(key).cloned()
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, value: Option<PathBuf>) {
        if self.entries.contains_key(&key) {
            self.order.retain(|k| k != &key);
        } else if self.entries.len() >= self.capacity {
            // Evict oldest
            if let Some(oldest) = self.order.first().cloned() {
                self.order.remove(0);
                self.entries.remove(&oldest);
            }
        }
        self.order.push(key.clone());
        self.entries.insert(key, value);
    }
}

impl BimiLruCache {
    /// Create a new LRU cache with the default capacity (500).
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(LruMap::new(LRU_CAPACITY)),
        }
    }

    /// Create a new LRU cache with a specific capacity.
    #[must_use]
    pub fn with_capacity(capacity: NonZeroUsize) -> Self {
        Self {
            cache: Mutex::new(LruMap::new(capacity.get())),
        }
    }

    /// Look up a domain in the in-memory cache.
    ///
    /// Returns `None` on cache miss. Returns `Some(None)` if the domain is
    /// known to have no BIMI. Returns `Some(Some(path))` with the logo path.
    pub fn get(&self, domain: &str) -> Option<Option<PathBuf>> {
        match self.cache.lock() {
            Ok(mut map) => map.get(domain),
            Err(_) => None,
        }
    }

    /// Insert a lookup result into the in-memory cache.
    pub fn insert(&self, domain: String, result: Option<PathBuf>) {
        if let Ok(mut map) = self.cache.lock() {
            map.insert(domain, result);
        }
    }
}

impl Default for BimiLruCache {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Cache warming
// ---------------------------------------------------------------------------

/// Maximum number of recent messages to scan for sender domains.
const WARM_MAX_MESSAGES: u32 = 1000;

/// Number of days to look back for recent messages.
const WARM_LOOKBACK_DAYS: i64 = 7;

/// Extract unique sender domains from recent messages that are not already
/// cached (or whose cache has expired).
fn domains_to_warm(conn: &Connection) -> Result<Vec<String>, String> {
    let cutoff = chrono::Utc::now().timestamp() - (WARM_LOOKBACK_DAYS * 24 * 3600);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT LOWER(SUBSTR(m.from_address, INSTR(m.from_address, '@') + 1)) AS domain \
             FROM messages m \
             WHERE m.from_address IS NOT NULL \
               AND m.from_address LIKE '%@%' \
               AND m.date > ?1 \
             ORDER BY domain \
             LIMIT ?2",
        )
        .map_err(|e| format!("prepare domain query: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![cutoff, WARM_MAX_MESSAGES], |row| {
            row.get::<_, String>("domain")
        })
        .map_err(|e| format!("query sender domains: {e}"))?;

    let mut domains = Vec::new();
    for row in rows {
        let domain = row.map_err(|e| format!("read domain row: {e}"))?;
        if domain.is_empty() || !domain.contains('.') {
            continue;
        }
        // Check if already cached and not expired
        match get_bimi_cache(conn, &domain) {
            Ok(Some(_)) => continue, // still valid
            Ok(None) | Err(_) => domains.push(domain),
        }
    }

    Ok(domains)
}

/// Pre-fetch BIMI logos for unique sender domains from recent messages.
///
/// This runs DNS lookups and SVG fetches concurrently (up to `max_concurrent`)
/// for domains not already in the cache. Cache warming is best-effort: failures
/// for individual domains are logged but do not abort the overall operation.
///
/// Returns the count of domains that were newly looked up (both positive and
/// negative results count).
pub async fn warm_bimi_cache(
    conn: &Connection,
    cache_dir: &Path,
    max_concurrent: usize,
) -> Result<usize, String> {
    let domains = domains_to_warm(conn)?;
    let total = domains.len();

    if total == 0 {
        debug!("BIMI cache warm: no domains to process");
        return Ok(0);
    }

    info!("BIMI cache warm: processing {total} domains (concurrency {max_concurrent})");

    // We need to collect results synchronously back to update the DB, so we
    // run the async lookups and collect (domain, result) pairs.
    type BimiResult = (String, Option<(PathBuf, String, Option<String>)>);
    let results: Vec<BimiResult> = stream::iter(domains)
        .map(|domain| {
            let cache_dir = cache_dir.to_path_buf();
            async move {
                let result = lookup_bimi_dns_and_fetch(&domain, &cache_dir).await;
                (domain, result)
            }
        })
        .buffer_unordered(max_concurrent)
        .collect()
        .await;

    let mut warmed = 0usize;
    for (domain, result) in &results {
        let now = chrono::Utc::now().timestamp();
        match result {
            Some((_path, logo_uri, authority_uri)) => {
                let expires = now + POSITIVE_TTL_SECS;
                if let Err(e) = upsert_bimi_cache(
                    conn,
                    domain,
                    true,
                    Some(logo_uri.as_str()),
                    authority_uri.as_deref(),
                    expires,
                ) {
                    warn!("failed to cache BIMI warm result for {domain}: {e}");
                }
            }
            None => {
                cache_negative(domain, conn);
            }
        }
        warmed += 1;
    }

    info!("BIMI cache warm: completed {warmed} domains");
    Ok(warmed)
}

/// Perform a DNS lookup and SVG fetch for a single domain (no DB caching).
/// Returns `Some((png_path, logo_uri, authority_uri))` on success, `None` if
/// no BIMI record or fetch failure.
async fn lookup_bimi_dns_and_fetch(
    domain: &str,
    cache_dir: &Path,
) -> Option<(PathBuf, String, Option<String>)> {
    let record = match lookup_bimi_dns(domain).await {
        Ok(Some(r)) => r,
        Ok(None) => {
            debug!("BIMI warm: no DNS record for {domain}");
            return None;
        }
        Err(e) => {
            debug!("BIMI warm: DNS error for {domain}: {e}");
            return None;
        }
    };

    let logo_uri = match record.logo_uri {
        Some(ref uri) if !uri.is_empty() => uri.clone(),
        _ => {
            debug!("BIMI warm: no logo URI for {domain}");
            return None;
        }
    };

    let path = cache_path(cache_dir, &logo_uri);
    if path.exists() {
        return Some((path, logo_uri, record.authority_uri));
    }

    let svg_data = match fetch_and_validate_svg(&logo_uri).await {
        Ok(data) => data,
        Err(e) => {
            debug!("BIMI warm: SVG fetch failed for {domain}: {e}");
            return None;
        }
    };

    if let Err(e) = rasterize_svg_to_png(&svg_data, &path) {
        debug!("BIMI warm: rasterize failed for {domain}: {e}");
        return None;
    }

    Some((path, logo_uri, record.authority_uri))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bimi_record_full() {
        let txt = "v=BIMI1; l=https://example.com/logo.svg; a=https://example.com/cert.pem";
        let rec = parse_bimi_record(txt).expect("should parse");
        assert_eq!(
            rec.logo_uri.as_deref(),
            Some("https://example.com/logo.svg")
        );
        assert_eq!(
            rec.authority_uri.as_deref(),
            Some("https://example.com/cert.pem")
        );
    }

    #[test]
    fn test_parse_bimi_record_logo_only() {
        let txt = "v=BIMI1; l=https://example.com/logo.svg;";
        let rec = parse_bimi_record(txt).expect("should parse");
        assert_eq!(
            rec.logo_uri.as_deref(),
            Some("https://example.com/logo.svg")
        );
        assert!(rec.authority_uri.is_none());
    }

    #[test]
    fn test_parse_bimi_record_no_logo() {
        let txt = "v=BIMI1; l=; a=;";
        let rec = parse_bimi_record(txt).expect("should parse");
        assert!(rec.logo_uri.is_none());
        assert!(rec.authority_uri.is_none());
    }

    #[test]
    fn test_parse_bimi_record_invalid_version() {
        assert!(parse_bimi_record("v=BIMI2; l=https://x.com/l.svg").is_none());
        assert!(parse_bimi_record("not a bimi record").is_none());
    }

    #[test]
    fn test_dmarc_passed() {
        assert!(dmarc_passed(
            "mx.google.com; dkim=pass; spf=pass; dmarc=pass"
        ));
        assert!(!dmarc_passed(
            "mx.google.com; dkim=pass; spf=pass; dmarc=fail"
        ));
        assert!(!dmarc_passed("mx.google.com; dkim=pass; spf=pass"));
    }

    #[test]
    fn test_organizational_domain() {
        assert_eq!(
            organizational_domain("mail.example.com"),
            Some("example.com")
        );
        assert_eq!(
            organizational_domain("sub.mail.example.com"),
            Some("mail.example.com")
        );
        assert_eq!(organizational_domain("example.com"), None);
        assert_eq!(organizational_domain("localhost"), None);
    }

    #[test]
    fn test_decode_bimi_indicator() {
        let svg = b"<svg>test</svg>";
        let encoded = BASE64.encode(svg);
        let decoded = decode_bimi_indicator(&encoded).expect("should decode");
        assert_eq!(decoded, svg);

        assert!(decode_bimi_indicator("").is_none());
        assert!(decode_bimi_indicator("   ").is_none());
    }

    #[test]
    fn test_validate_svg_rejects_missing_baseprofile() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><circle/></svg>";
        assert!(validate_svg(svg).is_err());
    }

    #[test]
    fn test_validate_svg_accepts_tiny_ps() {
        let svg =
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" baseProfile=\"tiny-ps\"><circle/></svg>";
        assert!(validate_svg(svg).is_ok());
    }

    #[test]
    fn test_validate_svg_rejects_external_refs() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" baseProfile=\"tiny-ps\"><image href=\"https://evil.com/x.png\"/></svg>";
        assert!(validate_svg(svg).is_err());
    }

    #[test]
    fn test_cache_path_deterministic() {
        let dir = Path::new("/tmp/test-cache");
        let p1 = cache_path(dir, "https://example.com/logo.svg");
        let p2 = cache_path(dir, "https://example.com/logo.svg");
        assert_eq!(p1, p2);
        assert!(p1.starts_with("/tmp/test-cache/bimi/"));
        assert!(p1.extension().is_some_and(|e| e == "png"));
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode([0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode([0x00, 0xff]), "00ff");
    }

    #[test]
    fn test_lru_cache_basic() {
        let cache = BimiLruCache::new();

        // Miss on empty cache
        assert!(cache.get("example.com").is_none());

        // Insert positive
        cache.insert(
            "example.com".to_string(),
            Some(PathBuf::from("/tmp/logo.png")),
        );
        let result = cache.get("example.com");
        assert!(result.is_some());
        assert_eq!(
            result.as_ref().and_then(|r| r.as_ref()),
            Some(&PathBuf::from("/tmp/logo.png"))
        );

        // Insert negative
        cache.insert("nodomain.com".to_string(), None);
        let result = cache.get("nodomain.com");
        assert_eq!(result, Some(None));
    }

    #[test]
    fn test_lru_cache_eviction() {
        let cache = BimiLruCache::with_capacity(NonZeroUsize::new(2).expect("nonzero"));

        cache.insert("a.com".to_string(), None);
        cache.insert("b.com".to_string(), None);
        cache.insert("c.com".to_string(), None); // should evict a.com

        assert!(cache.get("a.com").is_none()); // evicted
        assert!(cache.get("b.com").is_some());
        assert!(cache.get("c.com").is_some());
    }

    #[test]
    fn test_lru_cache_access_refreshes() {
        let cache = BimiLruCache::with_capacity(NonZeroUsize::new(2).expect("nonzero"));

        cache.insert("a.com".to_string(), None);
        cache.insert("b.com".to_string(), None);

        // Access a.com to make it most-recently-used
        let _ = cache.get("a.com");

        // Insert c.com — should evict b.com (oldest), not a.com
        cache.insert("c.com".to_string(), None);

        assert!(cache.get("a.com").is_some());
        assert!(cache.get("b.com").is_none()); // evicted
        assert!(cache.get("c.com").is_some());
    }
}
