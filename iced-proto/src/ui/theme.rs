use iced::Color;

// Dark theme colors matched from the Tauri screenshot
pub const BG_BASE: Color = Color::from_rgb(0.059, 0.090, 0.165); // #0f172a
pub const BG_SIDEBAR: Color = Color::from_rgb(0.067, 0.082, 0.145); // #111525
pub const BG_SURFACE: Color = Color::from_rgb(0.086, 0.110, 0.192); // #161c31
pub const BG_ELEVATED: Color = Color::from_rgb(0.110, 0.141, 0.235); // #1c243c
pub const BG_HOVER: Color = Color::from_rgb(0.133, 0.165, 0.263); // #222a43
pub const BG_SELECTED: Color = Color::from_rgb(0.153, 0.180, 0.290); // #272e4a

pub const TEXT_PRIMARY: Color = Color::from_rgb(0.945, 0.961, 0.980); // #f1f5fa
pub const TEXT_SECONDARY: Color = Color::from_rgb(0.690, 0.733, 0.804); // #b0bbcd
pub const TEXT_TERTIARY: Color = Color::from_rgb(0.502, 0.553, 0.647); // #808da5

pub const ACCENT: Color = Color::from_rgb(0.384, 0.400, 0.945); // #6266f1
pub const ACCENT_DIM: Color = Color::from_rgb(0.384, 0.400, 0.945); // same, for subtle uses
pub const DANGER: Color = Color::from_rgb(0.863, 0.149, 0.149); // #dc2626
pub const WARNING: Color = Color::from_rgb(0.851, 0.467, 0.024); // #d97706
pub const SUCCESS: Color = Color::from_rgb(0.020, 0.588, 0.412); // #059669

pub const BORDER: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.08);
pub const BORDER_SUBTLE: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.04);

// Avatar colors for initials
pub const AVATAR_COLORS: &[Color] = &[
    Color::from_rgb(0.384, 0.400, 0.945), // indigo
    Color::from_rgb(0.020, 0.588, 0.412), // green
    Color::from_rgb(0.863, 0.149, 0.149), // red
    Color::from_rgb(0.851, 0.467, 0.024), // amber
    Color::from_rgb(0.608, 0.318, 0.878), // purple
    Color::from_rgb(0.059, 0.522, 0.780), // cyan
    Color::from_rgb(0.878, 0.318, 0.518), // pink
    Color::from_rgb(0.180, 0.620, 0.220), // emerald
];

pub fn avatar_color(name: &str) -> Color {
    let hash: usize = name.bytes().map(|b| b as usize).sum();
    AVATAR_COLORS[hash % AVATAR_COLORS.len()]
}

pub fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}
