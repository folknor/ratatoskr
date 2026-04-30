use iced::Color;

use super::color::hsl_to_color;

const AVATAR_HUES: &[f32] = &[
    260.0, // indigo
    160.0, // green
    25.0,  // red-orange
    45.0,  // amber
    290.0, // purple
    195.0, // cyan
    340.0, // pink
    130.0, // emerald
];

pub fn avatar_color(name: &str) -> Color {
    let hash: usize = name.bytes().map(|b| b as usize).sum();
    let hue = AVATAR_HUES[hash % AVATAR_HUES.len()];
    hsl_to_color(hue, 0.65, 0.55)
}

pub fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}
