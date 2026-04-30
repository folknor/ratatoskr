use iced::Theme;
use iced::theme::palette::Seed;
use serde::Deserialize;

use super::color::hex_to_color;

pub struct ThemeEntry {
    pub name: &'static str,
    pub palette: Seed,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry {
        name: "Light",
        palette: Seed::LIGHT,
    },
    ThemeEntry {
        name: "Dark",
        palette: Seed::DARK,
    },
    ThemeEntry {
        name: "Dracula",
        palette: Seed::DRACULA,
    },
    ThemeEntry {
        name: "Nord",
        palette: Seed::NORD,
    },
    ThemeEntry {
        name: "Solarized Light",
        palette: Seed::SOLARIZED_LIGHT,
    },
    ThemeEntry {
        name: "Solarized Dark",
        palette: Seed::SOLARIZED_DARK,
    },
    ThemeEntry {
        name: "Gruvbox Light",
        palette: Seed::GRUVBOX_LIGHT,
    },
    ThemeEntry {
        name: "Gruvbox Dark",
        palette: Seed::GRUVBOX_DARK,
    },
    ThemeEntry {
        name: "Catppuccin Latte",
        palette: Seed::CATPPUCCIN_LATTE,
    },
    ThemeEntry {
        name: "Catppuccin Frappé",
        palette: Seed::CATPPUCCIN_FRAPPE,
    },
    ThemeEntry {
        name: "Catppuccin Macchiato",
        palette: Seed::CATPPUCCIN_MACCHIATO,
    },
    ThemeEntry {
        name: "Catppuccin Mocha",
        palette: Seed::CATPPUCCIN_MOCHA,
    },
    ThemeEntry {
        name: "Tokyo Night",
        palette: Seed::TOKYO_NIGHT,
    },
    ThemeEntry {
        name: "Tokyo Night Storm",
        palette: Seed::TOKYO_NIGHT_STORM,
    },
    ThemeEntry {
        name: "Tokyo Night Light",
        palette: Seed::TOKYO_NIGHT_LIGHT,
    },
    ThemeEntry {
        name: "Kanagawa Wave",
        palette: Seed::KANAGAWA_WAVE,
    },
    ThemeEntry {
        name: "Kanagawa Lotus",
        palette: Seed::KANAGAWA_LOTUS,
    },
    ThemeEntry {
        name: "Moonfly",
        palette: Seed::MOONFLY,
    },
    ThemeEntry {
        name: "Nightfly",
        palette: Seed::NIGHTFLY,
    },
    ThemeEntry {
        name: "Oxocarbon",
        palette: Seed::OXOCARBON,
    },
    ThemeEntry {
        name: "Ferra",
        palette: Seed::FERRA,
    },
];

pub fn theme_by_index(index: usize) -> Theme {
    let entry = &THEMES[index.min(THEMES.len() - 1)];
    Theme::custom(entry.name.to_string(), entry.palette)
}

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    colors: ThemeColors,
}

#[derive(Debug, Deserialize)]
struct ThemeColors {
    background: String,
    text: String,
    primary: String,
    success: String,
    warning: String,
    danger: String,
}

pub fn from_toml(content: &str) -> Result<Theme, toml::de::Error> {
    let file: ThemeFile = toml::from_str(content)?;
    let palette = Seed {
        background: hex_to_color(&file.colors.background),
        text: hex_to_color(&file.colors.text),
        primary: hex_to_color(&file.colors.primary),
        success: hex_to_color(&file.colors.success),
        warning: hex_to_color(&file.colors.warning),
        danger: hex_to_color(&file.colors.danger),
    };
    Ok(Theme::custom(
        file.name.unwrap_or_else(|| "Custom".into()),
        palette,
    ))
}
