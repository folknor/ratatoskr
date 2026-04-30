use iced::Color;

/// Mix two colors by ratio (0.0 = a, 1.0 = b).
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

pub fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    // Guard against short/malformed hex strings - fall back to mid-gray
    // rather than panicking on slice bounds.
    if hex.len() < 6 {
        return Color::from_rgb8(128, 128, 128);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(128);
    Color::from_rgb8(r, g, b)
}

pub(super) fn hsl_to_color(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    // h_prime = h / 60.0 where h is in [0, 360), so h_prime is in [0, 6).
    // Truncation to u32 yields 0..=5, which is the intended bucket index.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::from_rgb(r1 + m, g1 + m, b1 + m)
}
