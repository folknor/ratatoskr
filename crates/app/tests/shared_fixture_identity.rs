//! Shared-fixture drift tripwire.
//!
//! Some sync fixtures exist as byte-identical copies in TWO repos: ours
//! (`crates/app/tests/sync-fixtures/`, what brokkr feeds the spawned mock)
//! and saehrimnir's (`fixtures/`, what the mock's own tests pin behavior
//! against). Nothing structural keeps the copies in step, and a silent drift
//! means our gates and saehrimnir's tests quietly exercise different
//! scenarios.
//!
//! Why this is a file comparison and not a check against the mock's
//! `GET /test/fixture/identity` digest: brokkr passes the mock OUR fixture
//! file (`[ratatoskr] fixtures_dir`), so the running mock's digest is
//! computed over the very copy we would compare it to - a tautology. The
//! meaningful pair is our copy vs the saehrimnir REPO copy, reachable here
//! through the in-tree `./research/saehrimnir` working copy. That clone is
//! also exactly where drift originates: a side-quest edits the mock's copy
//! there and must mirror the change into `sync-fixtures/` by hand.
//!
//! On hosts without `./research/saehrimnir` the cross-check skips - drift
//! cannot be INTRODUCED on a machine that has only one of the trees.

use std::path::{Path, PathBuf};

/// Fixtures held byte-identical in both repos. Add any future shared
/// fixture here; the saehrimnir path is `fixtures/<name>` by convention.
const SHARED_FIXTURES: &[&str] = &["gmail-incremental.lua"];

fn ours(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/sync-fixtures")
        .join(name)
}

fn theirs(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../research/saehrimnir/fixtures")
        .join(name)
}

#[test]
fn shared_fixtures_match_the_saehrimnir_copies() {
    for name in SHARED_FIXTURES {
        let our_path = ours(name);
        let our_bytes = std::fs::read(&our_path)
            .unwrap_or_else(|e| panic!("shared fixture {} unreadable: {e}", our_path.display()));

        let their_path = theirs(name);
        if !their_path.exists() {
            // No research clone on this host; the drift pair does not exist
            // here, so there is nothing to compare.
            eprintln!(
                "skipping {name}: no saehrimnir research copy at {}",
                their_path.display()
            );
            continue;
        }
        let their_bytes = std::fs::read(&their_path)
            .unwrap_or_else(|e| panic!("research copy {} unreadable: {e}", their_path.display()));

        assert!(
            our_bytes == their_bytes,
            "shared fixture {name} has drifted between crates/app/tests/sync-fixtures/ and \
             research/saehrimnir/fixtures/. The copies are a byte-identical contract: our gates \
             and saehrimnir's own tests must exercise the same scenario. Decide which side is \
             right, mirror it to the other (the saehrimnir side goes through the side-quest \
             procedure in docs/side-quests.md), and re-run."
        );
    }
}
