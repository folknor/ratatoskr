use std::path::PathBuf;

#[test]
fn local_search_links_no_provider_surface() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest = cargo_toml::Manifest::from_path(&manifest_path)
        .unwrap_or_else(|e| panic!("parse {}: {e}", manifest_path.display()));
    let banned_exact = [
        "common",
        "gmail",
        "graph",
        "jmap",
        "imap",
        "provider-sync",
        "bifrost-sync",
        "bifrost-types",
    ];

    for dependencies in [
        &manifest.dependencies,
        &manifest.dev_dependencies,
        &manifest.build_dependencies,
    ] {
        assert_provider_free(dependencies, &banned_exact);
    }
    for target in manifest.target.values() {
        for dependencies in [
            &target.dependencies,
            &target.dev_dependencies,
            &target.build_dependencies,
        ] {
            assert_provider_free(dependencies, &banned_exact);
        }
    }
}

fn assert_provider_free(dependencies: &cargo_toml::DepsSet, banned_exact: &[&str]) {
    for (dependency, definition) in dependencies {
        let package = definition
            .detail()
            .and_then(|detail| detail.package.as_deref())
            .unwrap_or(dependency);
        assert!(
            !banned_exact.contains(&package) && !package.starts_with("bifrost-"),
            "search must remain a provider-free local Tantivy leaf; Cargo.toml has forbidden dependency `{package}` (declared as `{dependency}`)"
        );
    }
}
