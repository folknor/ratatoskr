//! Deterministic label color resolution.
//!
//! Labels synced from Gmail have explicit `color_bg`/`color_fg`. All other
//! providers store `None`. This module provides a hash-based fallback that
//! assigns a stable color from the 25-preset palette to any label.

pub mod preset_colors;

use preset_colors::all_presets;
use db::db::types::DbLabel;

/// Deterministic color assignment for a label that has no synced color.
///
/// Hashes the label name to produce a stable index into the 25-preset
/// palette. The `namespace` parameter (typically account_id) ensures
/// labels with the same name on different accounts can get different
/// colors if desired, but can be set to `""` for global consistency.
///
/// Returns `(bg_hex, fg_hex)`.
pub fn color_for_label(label_name: &str, namespace: &str) -> (&'static str, &'static str) {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    namespace.hash(&mut hasher);
    label_name.hash(&mut hasher);
    let presets = all_presets();
    // Truncation from u64 to usize on 32-bit targets is fine here — we only
    // need an arbitrary hash value to index into a 25-element table.
    #[allow(clippy::cast_possible_truncation)]
    let index = (hasher.finish() as usize) % presets.len();
    let (_, bg, fg) = presets[index];
    (bg, fg)
}

/// Resolve display colors for a label.
///
/// If the label has synced `color_bg`/`color_fg` (Gmail), return those.
/// Otherwise, deterministically assign from the preset palette.
pub fn resolve_label_color(label: &DbLabel) -> (&str, &str) {
    let result = match (&label.color_bg, &label.color_fg) {
        (Some(bg), Some(fg)) => (bg.as_str(), fg.as_str()),
        _ => color_for_label(&label.name, &label.account_id),
    };
    log::debug!(
        "Resolved label color: name={}, bg={}, fg={}",
        label.name,
        result.0,
        result.1,
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_color() {
        let (bg1, fg1) = color_for_label("Work", "acc-1");
        let (bg2, fg2) = color_for_label("Work", "acc-1");
        assert_eq!(bg1, bg2);
        assert_eq!(fg1, fg2);
    }

    #[test]
    fn different_namespace_can_differ() {
        let (bg1, _) = color_for_label("Work", "acc-1");
        let (bg2, _) = color_for_label("Work", "acc-2");
        // Not guaranteed to differ, but the function should not panic
        let _ = (bg1, bg2);
    }

    #[test]
    fn resolve_prefers_synced() {
        let label = DbLabel {
            id: "l1".to_string(),
            account_id: "acc-1".to_string(),
            name: "Important".to_string(),
            label_type: None,
            color_bg: Some("#ff0000".to_string()),
            color_fg: Some("#ffffff".to_string()),
            visible: true,
            sort_order: 0,
            imap_folder_path: None,
            imap_special_use: None,
        };
        let (bg, fg) = resolve_label_color(&label);
        assert_eq!(bg, "#ff0000");
        assert_eq!(fg, "#ffffff");
    }

    #[test]
    fn resolve_falls_back_to_hash() {
        let label = DbLabel {
            id: "l2".to_string(),
            account_id: "acc-1".to_string(),
            name: "Custom".to_string(),
            label_type: None,
            color_bg: None,
            color_fg: None,
            visible: true,
            sort_order: 0,
            imap_folder_path: None,
            imap_special_use: None,
        };
        let (bg, fg) = resolve_label_color(&label);
        // Should be a valid preset color (starts with #)
        assert!(bg.starts_with('#'));
        assert!(fg.starts_with('#'));
        // Should match the direct hash call
        let (expected_bg, expected_fg) = color_for_label("Custom", "acc-1");
        assert_eq!(bg, expected_bg);
        assert_eq!(fg, expected_fg);
    }
}
