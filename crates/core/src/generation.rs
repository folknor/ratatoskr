use std::marker::PhantomData;

/// A monotonically increasing counter for guarding async loads against
/// stale results. `next()` is the only way to obtain a token — it bumps
/// the counter and returns the token in one step. `is_current()` is the
/// only way to check freshness.
///
/// The phantom type `T` brands the counter so that tokens from different
/// counters are incompatible at compile time.
#[derive(Debug)]
pub struct GenerationCounter<T> {
    value: u64,
    _brand: PhantomData<T>,
}

impl<T> GenerationCounter<T> {
    pub fn new() -> Self {
        Self {
            value: 0,
            _brand: PhantomData,
        }
    }

    /// Bump the counter and return a token capturing the new value.
    /// This is the only way to obtain a token.
    #[must_use = "use the returned token, or `let _ = counter.next()` to invalidate without capturing"]
    pub fn next(&mut self) -> GenerationToken<T> {
        self.value = self.value.wrapping_add(1);
        GenerationToken(self.value, PhantomData)
    }

    /// Check if a token matches the current counter value.
    pub fn is_current(&self, token: GenerationToken<T>) -> bool {
        self.value == token.0
    }
}

impl<T> Default for GenerationCounter<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// An opaque token capturing a generation counter value at a point in time.
/// Carried through async tasks and Message variants to detect staleness.
///
/// Branded by `T` — a token from `GenerationCounter<Nav>` cannot be
/// checked against `GenerationCounter<Search>`.
pub struct GenerationToken<T>(u64, PhantomData<T>);

impl<T> std::fmt::Debug for GenerationToken<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("GenerationToken").field(&self.0).finish()
    }
}

impl<T> Clone for GenerationToken<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for GenerationToken<T> {}

impl<T> PartialEq for GenerationToken<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T> Eq for GenerationToken<T> {}

// ── Brand tags ──────────────────────────────────────────────

/// Brand for the navigation/accounts generation counter.
#[derive(Debug)]
pub enum Nav {}

/// Brand for the thread detail generation counter.
#[derive(Debug)]
pub enum ThreadDetail {}

/// Brand for the search results generation counter.
#[derive(Debug)]
pub enum Search {}

/// Brand for the pop-out window generation counter.
#[derive(Debug)]
pub enum PopOut {}

/// Brand for calendar event/calendar list loads.
#[derive(Debug)]
pub enum Calendar {}

/// Brand for command palette option loads.
#[derive(Debug)]
pub enum PaletteOptions {}

/// Brand for thread list typeahead results.
#[derive(Debug)]
pub enum Typeahead {}

/// Brand for add-account wizard async operations.
#[derive(Debug)]
pub enum AddAccount {}

/// Brand for compose contact autocomplete.
#[derive(Debug)]
pub enum Autocomplete {}

/// Brand for chat timeline loads.
#[derive(Debug)]
pub enum Chat {}
