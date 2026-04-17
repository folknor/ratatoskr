use display_info::DisplayInfo;

/// Standard "reference" DPI — Windows/macOS baseline for 100% scaling.
const REFERENCE_DPI: f32 = 96.0;

/// Below this physical DPI, don't apply any extra app scaling.
const HIDPI_THRESHOLD: f32 = 120.0;

/// Detect an appropriate default app scale factor by querying the primary
/// monitor's physical resolution and dimensions before the windowing toolkit
/// starts.
///
/// Returns 1.0 when:
/// - The compositor is already scaling (scale_factor > 1.0)
/// - The display is standard DPI (≤140 PPI)
/// - Detection fails for any reason
///
/// Returns a computed scale (rounded to nearest 0.25) when the display is
/// HiDPI but the compositor reports no scaling.
pub fn detect_default_scale() -> f32 {
    let displays = match DisplayInfo::all() {
        Ok(d) => d,
        Err(_) => return 1.0,
    };

    // If any monitor is already compositor-scaled, don't pile on.
    if displays.iter().any(|d| d.scale_factor > 1.0) {
        return 1.0;
    }

    // Use the lowest DPI across all connected monitors — safest default
    // so nothing ends up oversized on the smaller-DPI screen.
    let min_dpi = displays
        .iter()
        .filter_map(|d| {
            let (px, mm) = (d.width, u32::try_from(d.width_mm).unwrap_or(0));
            compute_dpi(px, mm)
        })
        .reduce(f32::min);

    let Some(dpi) = min_dpi else {
        return 1.0;
    };

    if dpi <= HIDPI_THRESHOLD {
        return 1.0;
    }

    // Round to nearest 0.25 step.
    let raw_scale = dpi / REFERENCE_DPI;
    (raw_scale * 4.0).round() / 4.0
}

/// Compute DPI from pixel count and physical millimetres.
/// Returns `None` if the physical dimension is zero or missing (some monitors
/// report 0mm via EDID).
fn compute_dpi(pixels: u32, mm: u32) -> Option<f32> {
    if mm == 0 {
        return None;
    }
    let inches = mm as f32 / 25.4;
    Some(pixels as f32 / inches)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn dpi_4k_27_inch() {
        // 27" 4K: ~3840px across ~597mm → ~163 DPI
        let dpi = compute_dpi(3840, 597).unwrap();
        assert!((dpi - 163.0).abs() < 2.0);
    }

    #[test]
    fn dpi_4k_32_inch() {
        // 32" 4K: ~3840px across ~708mm → ~138 DPI
        let dpi = compute_dpi(3840, 708).unwrap();
        assert!((dpi - 138.0).abs() < 2.0);
    }

    #[test]
    fn dpi_1080p_24_inch() {
        // 24" 1080p: ~1920px across ~531mm → ~92 DPI
        let dpi = compute_dpi(1920, 531).unwrap();
        assert!((dpi - 92.0).abs() < 2.0);
    }

    #[test]
    fn zero_mm_returns_none() {
        assert!(compute_dpi(3840, 0).is_none());
    }
}
