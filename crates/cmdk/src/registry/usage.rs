use std::collections::HashMap;

use crate::id::CommandId;

/// Tracks command usage counts for recency/frequency ranking.
///
/// Persistence is deferred to Slice 6 - the app layer will be responsible
/// for saving and restoring this data.
pub struct UsageTracker {
    counts: HashMap<CommandId, u32>,
}

impl UsageTracker {
    pub fn new() -> Self {
        Self {
            counts: HashMap::new(),
        }
    }

    pub fn record_usage(&mut self, id: CommandId) {
        *self.counts.entry(id).or_insert(0) += 1;
    }

    pub fn usage_count(&self, id: CommandId) -> u32 {
        self.counts.get(&id).copied().unwrap_or(0)
    }

    /// Serialize usage counts to a JSON-compatible map.
    /// Keys are the stable `CommandId::as_str()` identifiers.
    pub fn to_map(&self) -> HashMap<String, u32> {
        self.counts
            .iter()
            .map(|(id, count)| (id.as_str().to_string(), *count))
            .collect()
    }

    /// Load usage counts from a previously serialized map.
    /// Unknown command IDs are silently skipped.
    pub fn load_from_map(&mut self, map: &HashMap<String, u32>) {
        for (key, count) in map {
            if let Some(id) = CommandId::parse(key) {
                self.counts.insert(id, *count);
            }
        }
    }
}

impl Default for UsageTracker {
    fn default() -> Self {
        Self::new()
    }
}
