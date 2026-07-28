use std::path::{Path, PathBuf};

// B15e renamed the workspace alias `bifrost-jmap-new` back to `bifrost-jmap`
// when the retired external jmap-client git dep went away, so the two
// `bifrost_jmap::` tokens below now name BIFROST's own crate rather than the
// old client. That is deliberate and the lockdown still holds: account
// settings route through `AccountSettingsSurface` over the engine, never
// through a direct JMAP client module, whichever crate provides it.
const HAND_ROLLED_SETTINGS_TOKENS: [&str; 9] = [
    "bifrost_jmap::identity::IdentitySet",
    "bifrost_jmap::vacation_response",
    "/settings/sendAs",
    "/settings/vacation",
    "/me/mailboxSettings",
    "automaticRepliesSetting",
    "list_send_as",
    "update_send_as_signature",
    "GmailSendAs",
];

const ENGINE_SETTINGS_METHODS: [&str; 5] = [
    "identities_list",
    "identity_update",
    "vacation_get",
    "vacation_set",
    "quota_get",
];

#[test]
fn account_settings_route_through_bifrost() {
    let root = workspace_root();
    // B15e deleted `crates/jmap` outright, which subsumes the two narrower
    // assertions this test used to make (no `signatures.rs`, no
    // `pub mod signatures;` in its lib). Pin the stronger fact instead.
    let legacy_jmap = root.join("crates/jmap");
    assert!(
        !legacy_jmap.exists(),
        "{} restores the legacy JMAP client crate deleted in B15e; providers route through bifrost-jmap",
        legacy_jmap.display(),
    );

    let auto_responses = read(&root.join("crates/core/src/auto_responses.rs"));
    for function in [
        "fetch_graph_auto_response",
        "push_graph_auto_response",
        "fetch_gmail_auto_response",
        "push_gmail_auto_response",
        "fetch_jmap_auto_response",
        "push_jmap_auto_response",
    ] {
        assert!(
            !auto_responses.contains(function),
            "crates/core/src/auto_responses.rs restores retired provider vacation function `{function}`; use AccountSettingsSurface instead",
        );
    }

    let allowed = root.join("crates/service/src/bifrost/settings.rs");
    for path in rust_files_under(&root.join("crates")) {
        if !path.components().any(|part| part.as_os_str() == "src") {
            continue;
        }
        let raw = strip_test_modules(&read(&path));
        for needle in HAND_ROLLED_SETTINGS_TOKENS {
            assert!(
                !raw.contains(needle),
                "{} contains retired hand-rolled account-settings token `{needle}`; route account settings through AccountSettingsSurface",
                path.display(),
            );
        }
        for method in ENGINE_SETTINGS_METHODS {
            let needle = format!(".engine\n            .{method}(");
            let found = raw.matches(&needle).count();
            let permitted = usize::from(path == allowed);
            assert!(
                found == permitted,
                "{} contains {found} engine `{method}` call(s), allowed {permitted}; AccountSettingsSurface is the sole engine settings entry point",
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
