/// A monotonically increasing counter for guarding async loads against
/// stale results. `next()` bumps and returns a token in one step,
/// making it hard to forget the bump. `is_current()` is the only way
/// to check freshness. `current()` reads without bumping — use only
/// when capturing after an earlier `next()` in the same logical operation.
#[derive(Debug)]
pub struct GenerationCounter {
    value: u64,
}

impl GenerationCounter {
    pub fn new() -> Self {
        Self { value: 0 }
    }

    /// Bump the counter and return a token capturing the new value.
    /// Prefer `let token = counter.next()` over `counter.next(); counter.current()`.
    #[must_use = "use the returned token, or `let _ = counter.next()` to invalidate without capturing"]
    pub fn next(&mut self) -> GenerationToken {
        self.value = self.value.wrapping_add(1);
        GenerationToken(self.value)
    }

    /// Get a token for the current value without bumping.
    /// Use this when capturing into an async task after an earlier `next()`.
    pub fn current(&self) -> GenerationToken {
        GenerationToken(self.value)
    }

    /// Check if a token matches the current counter value.
    pub fn is_current(&self, token: GenerationToken) -> bool {
        self.value == token.0
    }
}

impl Default for GenerationCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// An opaque token capturing a generation counter value at a point in time.
/// Carried through async tasks and Message variants to detect staleness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenerationToken(u64);
