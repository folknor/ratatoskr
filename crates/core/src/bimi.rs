use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex;

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_cache_basic() {
        let cache = BimiLruCache::new();

        // Miss on empty cache
        assert!(cache.get("example.com").is_none());

        // Insert positive
        cache.insert(
            "example.com".to_string(),
            Some(PathBuf::from("cache/logo.png")),
        );
        let result = cache.get("example.com");
        assert!(result.is_some());
        assert_eq!(
            result.as_ref().and_then(|r| r.as_ref()),
            Some(&PathBuf::from("cache/logo.png"))
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

        // Insert c.com - should evict b.com (oldest), not a.com
        cache.insert("c.com".to_string(), None);

        assert!(cache.get("a.com").is_some());
        assert!(cache.get("b.com").is_none()); // evicted
        assert!(cache.get("c.com").is_some());
    }
}
