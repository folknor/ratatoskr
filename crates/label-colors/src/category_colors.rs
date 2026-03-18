/// Unified color model for email categories across providers.
///
/// Exchange's 25 preset colors serve as the canonical palette. Other providers
/// (Gmail labels, JMAP keywords) can map arbitrary hex colors to the nearest
/// Exchange preset using Euclidean distance in RGB space.

/// (preset_name, background_hex, foreground_hex)
const PRESETS: &[(&str, &str, &str)] = &[
    ("preset0", "#e74c3c", "#ffffff"),  // Red
    ("preset1", "#e67e22", "#ffffff"),  // Orange
    ("preset2", "#8b4513", "#ffffff"),  // Brown
    ("preset3", "#f1c40f", "#000000"),  // Yellow
    ("preset4", "#2ecc71", "#ffffff"),  // Green
    ("preset5", "#1abc9c", "#ffffff"),  // Teal
    ("preset6", "#808000", "#ffffff"),  // Olive
    ("preset7", "#3498db", "#ffffff"),  // Blue
    ("preset8", "#9b59b6", "#ffffff"),  // Purple
    ("preset9", "#c0392b", "#ffffff"),  // Cranberry
    ("preset10", "#708090", "#ffffff"), // Steel
    ("preset11", "#4a5568", "#ffffff"), // DarkSteel
    ("preset12", "#95a5a6", "#000000"), // Gray
    ("preset13", "#636e72", "#ffffff"), // DarkGray
    ("preset14", "#2d3436", "#ffffff"), // Black
    ("preset15", "#8b0000", "#ffffff"), // DarkRed
    ("preset16", "#d35400", "#ffffff"), // DarkOrange
    ("preset17", "#5d3a1a", "#ffffff"), // DarkBrown
    ("preset18", "#b8860b", "#ffffff"), // DarkYellow
    ("preset19", "#1e7e34", "#ffffff"), // DarkGreen
    ("preset20", "#0e6655", "#ffffff"), // DarkTeal
    ("preset21", "#556b2f", "#ffffff"), // DarkOlive
    ("preset22", "#1a5276", "#ffffff"), // DarkBlue
    ("preset23", "#6c3483", "#ffffff"), // DarkPurple
    ("preset24", "#922b21", "#ffffff"), // DarkCranberry
];

/// Parse a hex color string (with or without `#` prefix) into (r, g, b).
/// Returns `None` for malformed input.
fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Squared Euclidean distance between two RGB colors.
fn color_distance_sq(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    (dr * dr + dg * dg + db * db) as u32
}

/// Look up a preset by name, returning `(bg_hex, fg_hex)`.
///
/// The preset name is matched case-insensitively.
///
/// ```
/// # use ratatoskr_label_colors::category_colors::preset_to_hex;
/// assert_eq!(preset_to_hex("preset0"), Some(("#e74c3c", "#ffffff")));
/// assert_eq!(preset_to_hex("Preset7"), Some(("#3498db", "#ffffff")));
/// assert_eq!(preset_to_hex("unknown"), None);
/// ```
pub fn preset_to_hex(preset: &str) -> Option<(&'static str, &'static str)> {
    let lower = preset.to_ascii_lowercase();
    PRESETS
        .iter()
        .find(|(name, _, _)| *name == lower)
        .map(|(_, bg, fg)| (*bg, *fg))
}

/// Find the nearest Exchange preset for an arbitrary hex background color.
///
/// Returns the preset name (e.g. `"preset7"`) of the closest match by
/// Euclidean distance in RGB space, or `None` if `bg_hex` is malformed.
///
/// ```
/// # use ratatoskr_label_colors::category_colors::nearest_exchange_preset;
/// // Exact match
/// assert_eq!(nearest_exchange_preset("#e74c3c"), Some("preset0"));
/// // Close to blue
/// assert_eq!(nearest_exchange_preset("#3366cc"), Some("preset22"));
/// ```
pub fn nearest_exchange_preset(bg_hex: &str) -> Option<&'static str> {
    let target = parse_hex(bg_hex)?;

    let mut best_name: &str = PRESETS[0].0;
    let mut best_dist = u32::MAX;

    for &(name, bg, _) in PRESETS {
        // PRESETS hex values are known-good, unwrap is safe here.
        let preset_rgb = parse_hex(bg).expect("invalid preset hex in PRESETS table");
        let dist = color_distance_sq(target, preset_rgb);
        if dist < best_dist {
            best_dist = dist;
            best_name = name;
            if dist == 0 {
                break;
            }
        }
    }

    Some(best_name)
}

/// Return the full preset table for iteration.
///
/// Each entry is `(preset_name, bg_hex, fg_hex)`.
pub fn all_presets() -> &'static [(&'static str, &'static str, &'static str)] {
    PRESETS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_to_hex_exact() {
        let (bg, fg) = preset_to_hex("preset0").expect("should find preset0");
        assert_eq!(bg, "#e74c3c");
        assert_eq!(fg, "#ffffff");
    }

    #[test]
    fn preset_to_hex_case_insensitive() {
        assert!(preset_to_hex("Preset3").is_some());
        assert!(preset_to_hex("PRESET3").is_some());
    }

    #[test]
    fn preset_to_hex_yellow_fg_is_black() {
        let (_, fg) = preset_to_hex("preset3").expect("should find preset3");
        assert_eq!(fg, "#000000");
    }

    #[test]
    fn preset_to_hex_unknown() {
        assert!(preset_to_hex("preset99").is_none());
        assert!(preset_to_hex("garbage").is_none());
    }

    #[test]
    fn nearest_exact_match() {
        // Each preset should map to itself
        for &(name, bg, _) in PRESETS {
            assert_eq!(
                nearest_exchange_preset(bg),
                Some(name),
                "exact match failed for {name}"
            );
        }
    }

    #[test]
    fn nearest_without_hash() {
        assert_eq!(nearest_exchange_preset("e74c3c"), Some("preset0"));
    }

    #[test]
    fn nearest_close_to_blue() {
        // #3366cc is close to DarkBlue (#1a5276) or Blue (#3498db)
        let result = nearest_exchange_preset("#3366cc").expect("valid hex");
        // Should be one of the blue presets
        assert!(
            result == "preset7" || result == "preset22",
            "expected a blue preset, got {result}"
        );
    }

    #[test]
    fn nearest_pure_white_maps_to_gray() {
        // White should map to the lightest preset (Gray #95a5a6)
        let result = nearest_exchange_preset("#ffffff").expect("valid hex");
        assert_eq!(result, "preset12", "white should be nearest to Gray");
    }

    #[test]
    fn nearest_pure_black_maps_to_black_preset() {
        let result = nearest_exchange_preset("#000000").expect("valid hex");
        assert_eq!(result, "preset14", "black should be nearest to Black preset");
    }

    #[test]
    fn nearest_invalid_hex() {
        assert!(nearest_exchange_preset("not-a-color").is_none());
        assert!(nearest_exchange_preset("#ff").is_none());
        assert!(nearest_exchange_preset("").is_none());
    }

    #[test]
    fn all_presets_has_25_entries() {
        assert_eq!(all_presets().len(), 25);
    }

    #[test]
    fn parse_hex_valid() {
        assert_eq!(parse_hex("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some((0, 255, 0)));
    }

    #[test]
    fn parse_hex_invalid() {
        assert!(parse_hex("#gg0000").is_none());
        assert!(parse_hex("#ff00").is_none());
    }
}
