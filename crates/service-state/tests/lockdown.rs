//! Global write-half lockdown gate: app must not reach service-state.
//!
//! Every UI-side write surface in `crates/app/src/` routes through a
//! Service IPC. Phase 6a/6b relocated the bulk of those surfaces;
//! Phase 6c relocated calendar event mutations; Phase 6d-A
//! relocated the contacts pipeline and deleted the last allow-listed
//! writable-connection accessor (`Db::phase_6c_pending_write_state`)
//! along with the `app.action_ctx` field that consumed it.
//!
//! Phase 6b's direct-dep check + Phase 6c-11's `app -> cal ->
//! service-state` transitive check closed the known UI-reachable
//! write escape paths. The strict transitive variant now closes the
//! remaining Cargo-graph gap: `app` may not reach `service-state`
//! through any path-dep chain.
//!
//! Why a Cargo.toml lint instead of a Rust visibility flip: the
//! service crate (also a separate crate from `service-state`)
//! legitimately needs to construct write-state instances for boot.
//! Flipping the constructors to `pub(crate)` would block service's
//! boot path, and there is no Rust visibility token that says
//! "visible to the service crate but not the app crate."
//! Cargo-toml-level enforcement is the practical equivalent.

use std::path::PathBuf;

#[test]
fn app_crate_must_not_directly_depend_on_service_state() {
    let app_cargo = workspace_path("crates/app/Cargo.toml");
    let raw = std::fs::read_to_string(&app_cargo)
        .unwrap_or_else(|e| panic!("read {}: {e}", app_cargo.display()));
    let manifest: toml::Value = toml::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse {}: {e}", app_cargo.display()));

    let deps = manifest
        .get("dependencies")
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| panic!("crates/app/Cargo.toml: no [dependencies] table"));
    assert!(
        !deps.contains_key("service-state"),
        "crates/app/Cargo.toml lists service-state as a direct dependency. Phase 6b's \
         global write-half lockdown forbids this: any write-state construction must \
         happen Service-side. Re-route through a Service IPC instead.",
    );

    let dev_deps = manifest
        .get("dev-dependencies")
        .and_then(|v| v.as_table());
    if let Some(table) = dev_deps {
        assert!(
            !table.contains_key("service-state"),
            "crates/app/Cargo.toml lists service-state as a direct dev-dependency. \
             Tests that need write access must run Service-side or use the IPC harness.",
        );
    }
}

#[test]
fn service_crate_can_still_construct_write_state() {
    // Sanity: the service crate's Cargo.toml MUST keep service-state
    // as a direct dep (the boot path constructs the write halves).
    // If a future refactor drops the dep, the boot path falls apart;
    // this test catches the regression early.
    let svc_cargo = workspace_path("crates/service/Cargo.toml");
    let raw = std::fs::read_to_string(&svc_cargo)
        .unwrap_or_else(|e| panic!("read {}: {e}", svc_cargo.display()));
    let manifest: toml::Value = toml::from_str(&raw)
        .unwrap_or_else(|e| panic!("parse {}: {e}", svc_cargo.display()));
    let deps = manifest
        .get("dependencies")
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| panic!("crates/service/Cargo.toml: no [dependencies] table"));
    assert!(
        deps.contains_key("service-state"),
        "crates/service/Cargo.toml lost its service-state dependency; the boot path \
         cannot construct WriteDbState without it.",
    );
}

fn workspace_path(suffix: &str) -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `crates/service-state`; walk two levels
    // up to reach the workspace root.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| panic!("unexpected manifest dir: {}", manifest_dir.display()));
    workspace_root.join(suffix)
}

/// Phase 6c-11: cal-out-of-app lockdown (the 6b-deferred check, refocused).
///
/// This test keeps the Phase 6c regression class visible: the
/// `app -> cal -> service-state` path that 6b documented as the
/// meaningful UI-reachable writer-half escape. Phase 6c-10 dropped
/// the `cal` dep from `app/Cargo.toml`; this test asserts it stays
/// dropped. The strict `app -> ... -> service-state` blackout is
/// enforced by the sibling test below.
///
/// Strategy is deliberately schema-light: parses each Cargo.toml,
/// builds a path-dep adjacency map, and walks from `app` to check
/// reachability of `cal`. No `cargo metadata` subprocess, no JSON
/// schema dependence; failure names the chain that re-introduces
/// the regression.
#[test]
fn app_crate_must_not_transitively_depend_on_cal() {
    let crates_dir = workspace_path("crates");
    let entries = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", crates_dir.display()));

    // Build name -> [dep_name, ...] adjacency from each Cargo.toml.
    let mut graph: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("dir entry: {e}"));
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let parsed: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", manifest.display()));
        let crate_name = parsed
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_else(|| panic!("no [package].name in {}", manifest.display()))
            .to_string();
        let mut deps = Vec::new();
        for table_key in &["dependencies", "dev-dependencies"] {
            if let Some(table) = parsed.get(*table_key).and_then(|v| v.as_table()) {
                for (name, value) in table {
                    // Only consider path-deps (workspace-local).
                    let is_path_dep = value
                        .as_table()
                        .map(|t| t.contains_key("path"))
                        .unwrap_or(false);
                    if is_path_dep {
                        deps.push(name.clone());
                    }
                }
            }
        }
        graph.insert(crate_name, deps);
    }

    // BFS from `app` looking for `cal`, blocking descent through
    // `service` (cal -> service is a legitimate Service-side edge;
    // the lockdown is about UI-side reachability).
    let blessed: std::collections::HashSet<&str> = ["service"].iter().copied().collect();
    let target = "cal";
    let start = "app";
    let mut parent: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut queue: std::collections::VecDeque<String> =
        std::collections::VecDeque::new();
    let mut visited: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    queue.push_back(start.to_string());
    visited.insert(start.to_string());
    while let Some(current) = queue.pop_front() {
        if current == target {
            // Walk parents back to `app` to build the chain.
            let mut chain: Vec<String> = vec![current.clone()];
            let mut cursor = current;
            while let Some(p) = parent.get(&cursor).cloned() {
                chain.push(p.clone());
                cursor = p;
            }
            chain.reverse();
            panic!(
                "Phase 6c-11 transitive lockdown failed: app reaches cal via {}.\n\
                 Phase 6c-10 closed the `app -> cal -> ...` path. Re-introducing it \
                 risks `cal::actions::*` writing locally from UI source files - the \
                 regression class Phase 6c relocated. Re-route through the \
                 `cal_action.execute_plan` IPC instead.",
                chain.join(" -> "),
            );
        }
        if blessed.contains(current.as_str()) {
            // Don't descend through `service` - it legitimately uses
            // cal. The hazard is UI-reachability of cal, not the
            // existence of the cal crate.
            continue;
        }
        if let Some(deps) = graph.get(&current) {
            for dep in deps {
                if visited.insert(dep.clone()) {
                    parent.insert(dep.clone(), current.clone());
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    // No path found - lockdown holds.
}

#[test]
fn app_crate_must_not_transitively_depend_on_service_state() {
    let crates_dir = workspace_path("crates");
    let entries = std::fs::read_dir(&crates_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", crates_dir.display()));

    let mut graph: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("dir entry: {e}"));
        let manifest = entry.path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let raw = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let parsed: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", manifest.display()));
        let crate_name = parsed
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or_else(|| panic!("no [package].name in {}", manifest.display()))
            .to_string();
        let mut deps = Vec::new();
        for table_key in &["dependencies", "dev-dependencies"] {
            if let Some(table) = parsed.get(*table_key).and_then(|v| v.as_table()) {
                for (name, value) in table {
                    let is_path_dep = value
                        .as_table()
                        .map(|t| t.contains_key("path"))
                        .unwrap_or(false);
                    if is_path_dep {
                        deps.push(name.clone());
                    }
                }
            }
        }
        graph.insert(crate_name, deps);
    }

    let target = "service-state";
    let start = "app";
    let mut parent: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut queue: std::collections::VecDeque<String> =
        std::collections::VecDeque::new();
    let mut visited: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    queue.push_back(start.to_string());
    visited.insert(start.to_string());
    while let Some(current) = queue.pop_front() {
        if current == target {
            let mut chain: Vec<String> = vec![current.clone()];
            let mut cursor = current;
            while let Some(p) = parent.get(&cursor).cloned() {
                chain.push(p.clone());
                cursor = p;
            }
            chain.reverse();
            panic!(
                "Strict service-state lockdown failed: app reaches service-state via {}.\n\
                 UI code must route durable writes through Service IPC. Move writer-half \
                 dependencies behind the service crate or provider-sync instead.",
                chain.join(" -> "),
            );
        }
        if let Some(deps) = graph.get(&current) {
            for dep in deps {
                if visited.insert(dep.clone()) {
                    parent.insert(dep.clone(), current.clone());
                    queue.push_back(dep.clone());
                }
            }
        }
    }
}
