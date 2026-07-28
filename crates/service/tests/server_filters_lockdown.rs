use std::path::{Path, PathBuf};

// `bifrost_jmap::sieve` names BIFROST's own crate since B15e renamed the
// `bifrost-jmap-new` alias. Deliberate: server filters route through
// `ServerFilterSurface` over the engine, never a direct JMAP client module.
const HAND_ROLLED_SIEVE_TOKENS: [&str; 6] = [
    "bifrost_jmap::sieve",
    "SieveScript",
    "SieveValidationResult",
    "server_supports_sieve",
    "create_sieve_script",
    "sieve_script_create",
];

const ENGINE_FILTER_CALLS: [&str; 5] = [
    ".filters_list(",
    ".filter_create(",
    ".filter_update(",
    ".filter_delete(",
    ".filter_validate(",
];

#[test]
fn server_filters_route_through_bifrost() {
    let root = workspace_root();
    // B15e deleted `crates/jmap` outright, which subsumes the narrower
    // "no `sieve.rs` / no `pub mod sieve;`" assertions this test used to make.
    let legacy_jmap = root.join("crates/jmap");
    assert!(
        !legacy_jmap.exists(),
        "{} restores the legacy JMAP client crate deleted in B15e; server filters route through ServerFilterSurface",
        legacy_jmap.display(),
    );

    let allowed = root.join("crates/service/src/bifrost/server_filters.rs");
    for path in rust_files_under(&root.join("crates")) {
        if !path.components().any(|part| part.as_os_str() == "src") {
            continue;
        }
        let raw = strip_test_modules(&read(&path));
        for needle in HAND_ROLLED_SIEVE_TOKENS {
            assert!(
                !raw.contains(needle),
                "{} contains retired hand-rolled Sieve token `{needle}`; route server filters through ServerFilterSurface",
                path.display(),
            );
        }
        for needle in ENGINE_FILTER_CALLS {
            let found = raw.matches(needle).count();
            let permitted = usize::from(path == allowed);
            assert!(
                found == permitted,
                "{} contains {found} `{needle}` call(s), allowed {permitted}; ServerFilterSurface is the sole engine filter entry point",
                path.display(),
            );
        }
    }
}

fn strip_test_modules(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(attribute) = rest.find("#[cfg(test)]") {
        output.push_str(&rest[..attribute]);
        let after_attribute = &rest[attribute + "#[cfg(test)]".len()..];
        let Some(open_brace) = after_attribute.find('{') else {
            break;
        };
        let mut depth = 0usize;
        let mut end = None;
        for (offset, character) in after_attribute[open_brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open_brace + offset + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        rest = &after_attribute[end..];
    }
    output.push_str(rest);
    output
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path)
                .unwrap_or_else(|error| panic!("read dir {}: {error}", path.display()))
            {
                pending.push(entry.expect("dir entry").path());
            }
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    files.sort();
    files
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/service must be nested under the workspace root")
        .to_path_buf()
}
