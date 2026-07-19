use std::path::{Path, PathBuf};

/// Provider / `SyncEngine` search verbs. None of these may appear anywhere in
/// the `rtsk` (core) or `service` source, save the single B8 GAL directory-
/// enumeration call. These are distinctive method names (no provider exposes
/// a `.query(`-shaped mail search), so a whole-tree substring walk catches a
/// bolted-on provider search without the false positives a blanket `.query(`
/// ban would drag in.
const PROVIDER_SEARCH_CALLS: [&str; 4] = [
    ".directory_search(",
    ".email_query(",
    ".uid_search(",
    ".list_threads(",
];

/// The one allow-listed provider-search call: B8's GAL directory enumeration
/// (`crates/service/src/handlers/gal.rs:167`), which pages the whole provider
/// directory into the 24h `gal_cache` with an EMPTY query - enumeration, not a
/// user mail search.
#[test]
fn search_pipeline_routes_local_only() {
    let root = workspace_root();

    // Whole-tree lockdown (the "walk-the-rust-files shape" of
    // db-read-lockdown's `db_read_raw_rusqlite_access_is_quarantined`): scan
    // EVERY .rs file under the two crates, so a provider search added to a new
    // module is caught too - not only the router as it stands today.
    for src in [
        root.join("crates/core/src"),
        root.join("crates/service/src"),
    ] {
        for path in rust_files_under(&src) {
            let raw = read(&path);
            let is_gal = path.ends_with("handlers/gal.rs");
            for needle in PROVIDER_SEARCH_CALLS {
                let allowed = usize::from(is_gal && needle == ".directory_search(");
                let found = raw.matches(needle).count();
                assert!(
                    found == allowed,
                    "{} contains {found} `{needle}` (allowed {allowed}). Mail search is local: \
                     route through the Tantivy and smart-folder SQL pipeline. The only permitted \
                     provider-search call is B8's GAL directory enumeration at \
                     crates/service/src/handlers/gal.rs.",
                    path.display(),
                );
            }
        }
    }

    // The SQL router itself must not grow a raw `.query(` against a provider
    // handle. `.query(` is the ordinary rusqlite read verb, so this needle is
    // scoped to the router file alone (known clean, and not a place a
    // legitimate raw SQL query belongs) rather than the whole tree, where it
    // would fire on every local query.
    let router = root.join("crates/core/src/search_pipeline.rs");
    let raw = read(&router);
    assert!(
        !raw.contains(".query("),
        "{} contains a raw `.query(`; the search router must never query a provider handle.",
        router.display(),
    );
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path)
                .unwrap_or_else(|e| panic!("read dir {}: {e}", path.display()))
            {
                pending.push(entry.expect("dir entry").path());
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            files.push(path);
        }
    }

    files.sort();
    files
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/core must be nested under the workspace root")
        .to_path_buf()
}
