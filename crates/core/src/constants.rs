//! Shared operational constants for the core crate and its dependents.
//!
//! Extract tunable parameters here so they're named, discoverable, and easy to
//! adjust without hunting through call sites.

use std::time::Duration;

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// HTTP request timeout for discovery probes (autoconfig, OIDC, JMAP
/// well-known, MX).  Applies to individual HTTP fetches, not the overall
/// discovery cascade (which has its own `OVERALL_TIMEOUT` in `discovery/mod.rs`).
pub const DISCOVERY_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// DAV (CalDAV / CardDAV)
// ---------------------------------------------------------------------------

/// HTTP request timeout for CalDAV and CardDAV operations.
pub const DAV_CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Query defaults
// ---------------------------------------------------------------------------

/// Re-export from `db` - the canonical definition lives at the storage layer.
pub use db::db::DEFAULT_QUERY_LIMIT;
