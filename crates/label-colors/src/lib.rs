//! Deterministic label color resolution.
//!
//! Labels may carry `user_color_*` or provider-synced `server_color_*`.
//! Labels without either pair get a hash-based fallback from the 25-preset
//! palette.

pub mod preset_colors;

use preset_colors::all_presets;

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
    #[allow(clippy::cast_possible_truncation)]
    let index = (hasher.finish() as usize) % presets.len();
    let (_, bg, fg) = presets[index];
    (bg, fg)
}

/// Resolve display colors for a label.
///
/// Resolution priority:
/// 1. User-selected color (`user_color_bg`/`user_color_fg` from the label row).
/// 2. Synced color (`server_color_bg`/`server_color_fg` from the label row).
/// 3. Hash fallback from the preset palette.
///
/// The `user_color` argument is the current row's user-selected color pair.
/// Callers pass `None` when no complete user pair is set.
pub fn resolve_label_color<'a>(
    name: &'a str,
    account_id: &'a str,
    user_color: Option<(&'a str, &'a str)>,
    server_color_bg: Option<&'a str>,
    server_color_fg: Option<&'a str>,
) -> (&'a str, &'a str) {
    let result = match (user_color, server_color_bg, server_color_fg) {
        (Some((bg, fg)), _, _) => (bg, fg),
        (None, Some(bg), Some(fg)) => (bg, fg),
        _ => color_for_label(name, account_id),
    };
    log::debug!(
        "Resolved label color: name={name}, bg={}, fg={}",
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
        let (bg, fg) = resolve_label_color(
            "Important",
            "acc-1",
            None,
            Some("#ff0000"),
            Some("#ffffff"),
        );
        assert_eq!(bg, "#ff0000");
        assert_eq!(fg, "#ffffff");
    }

    #[test]
    fn resolve_falls_back_to_hash() {
        let (bg, fg) = resolve_label_color("Custom", "acc-1", None, None, None);
        assert!(bg.starts_with('#'));
        assert!(fg.starts_with('#'));
        let (expected_bg, expected_fg) = color_for_label("Custom", "acc-1");
        assert_eq!(bg, expected_bg);
        assert_eq!(fg, expected_fg);
    }

    #[test]
    fn user_color_wins_over_synced() {
        let (bg, fg) = resolve_label_color(
            "Important",
            "acc-1",
            Some(("#00ff00", "#000000")),
            Some("#ff0000"),
            Some("#ffffff"),
        );
        assert_eq!(bg, "#00ff00");
        assert_eq!(fg, "#000000");
    }
}
