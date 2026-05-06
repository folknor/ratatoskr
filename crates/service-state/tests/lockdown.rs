//! Phase 6b global write-half lockdown gate.
//!
//! After Phase 6a-part-2 the app crate's only writable-connection
//! accessor on `Db` is `phase_6c_pending_write_state` (used by the
//! `cal::actions` ActionContext construction at `app.rs:336`,
//! removed in Phase 6c). Phase 6b layers a Cargo dependency check
//! on top: the app crate must not depend on `service-state`
//! directly. Without that direct dep, `WriteDbState`,
//! `BodyStoreWriteState`, `InlineImageStoreWriteState`, and
//! `SearchWriteHandle` are unreachable from `crates/app/src/`
//! regardless of the constructors' Rust visibility.
//!
//! The transitive variant of this check (`app -> cal -> service-state`)
//! is deferred to Phase 6c per `phase-6b-plan.md`'s arch-review
//! revision; that path closes when Phase 6c relocates
//! `cal::actions` Service-side and drops `cal` from
//! `app/Cargo.toml`.
//!
//! Why a Cargo.toml lint instead of a Rust visibility flip: the
//! service crate (also a separate crate from `service-state`)
//! legitimately needs to construct write-state instances for boot.
//! Flipping the constructors to `pub(crate)` would block service's
//! boot path, and there is no Rust visibility token that says
//! "visible to the service crate but not the app crate."
//! Cargo-toml-level enforcement is the practical equivalent for
//! phase-6b's invariant.

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
