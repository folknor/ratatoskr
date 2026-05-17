use std::path::PathBuf;

#[test]
fn repository_docs_catalog_is_clean() {
    let catalog = coverage::load_docs(workspace_root().join("reference"));

    if !catalog.diagnostics.is_empty() {
        let diagnostics = catalog
            .diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}:{}: {}",
                    diagnostic.file.display(),
                    diagnostic.line,
                    diagnostic.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        panic!("coverage doc catalog has diagnostics:\n{diagnostics}");
    }

    for expected in [
        "architecture.action_service_as_mutation_gate",
        "architecture.provider_trait_as_abstraction_layer",
        "architecture.generation_counters_for_async_safety",
        "architecture.folder_vs_label_semantics_are_explicit",
        "architecture.adding_a_new_email_action",
        "glossary.folders_labels.folder_rows_are_containers",
        "glossary.folders_labels.label_rows_are_tags",
        "glossary.folders_labels.storage_splits_folders_labels_and_groups",
        "glossary.folders_labels.label_identity_is_account_scoped",
        "glossary.folders_labels.system_folder_ids_are_canonical",
        "glossary.folders_labels.non_system_ids_keep_provider_prefixes",
        "glossary.folders_labels.provider_terms_translate_to_folder_label_semantics",
    ] {
        assert!(
            catalog
                .contracts
                .iter()
                .any(|contract| contract.id == expected),
            "missing smoke coverage contract `{expected}`",
        );
    }
}

#[test]
fn folders_labels_pilot_area_has_no_gaps() {
    let root = workspace_root();
    let lua_roots = vec![
        root.join("crates/app/tests/service-harness"),
        root.join("crates/app/tests/sync-harness"),
    ];
    let report = coverage::CoverageReport::build(root.join("reference"), &lua_roots);
    let area = "glossary.folders_labels";

    let uncovered = report
        .uncovered_contracts
        .iter()
        .filter(|contract| in_area(&contract.id, area))
        .map(|contract| contract.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        uncovered.is_empty(),
        "folders-labels pilot has uncovered contracts: {uncovered:?}",
    );

    let unknown = report
        .unknown_lua_claims
        .iter()
        .filter(|claim| in_area(&claim.id, area))
        .map(|claim| claim.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        unknown.is_empty(),
        "folders-labels pilot has unknown Lua claims: {unknown:?}",
    );
}

fn in_area(id: &str, area: &str) -> bool {
    id == area
        || id
            .strip_prefix(area)
            .map(|rest| rest.starts_with('.'))
            .unwrap_or(false)
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| panic!("unexpected manifest dir: {}", manifest_dir.display()))
        .to_path_buf()
}
