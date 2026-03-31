use std::borrow::Cow;
use std::sync::OnceLock;

use iced::Pixels;
use iced::font::{Style, Weight};
use iced::widget::text::LineHeight;

// ── Runtime font family ─────────────────────────────────

/// The detected UI font family name, or "Inter" as the bundled fallback.
static UI_FONT_FAMILY: OnceLock<&'static str> = OnceLock::new();

/// Set the UI font family detected from the system.
///
/// Must be called once before the iced app starts. If never called (or called
/// with `None`), the bundled Inter font is used.
pub fn set_system_ui_font(family: Option<String>) {
    let name: &'static str = match family {
        Some(f) if f != "Inter" => Box::leak(f.into_boxed_str()),
        _ => "Inter",
    };
    let _ = UI_FONT_FAMILY.set(name);
}

fn ui_family() -> &'static str {
    UI_FONT_FAMILY.get().copied().unwrap_or("Inter")
}

// ── Font constants ───────────────────────────────────────

pub const ICON: iced::Font = iced::Font::new("lucide");

pub fn text() -> iced::Font {
    iced::Font::new(ui_family())
}

pub fn text_bold() -> iced::Font {
    iced::Font {
        weight: Weight::Bold,
        ..iced::Font::new(ui_family())
    }
}

pub fn text_italic() -> iced::Font {
    iced::Font {
        style: Style::Italic,
        ..iced::Font::new(ui_family())
    }
}

pub fn text_bold_italic() -> iced::Font {
    iced::Font {
        weight: Weight::Bold,
        style: Style::Italic,
        ..iced::Font::new(ui_family())
    }
}

pub fn text_semibold() -> iced::Font {
    iced::Font {
        weight: Weight::Semibold,
        ..iced::Font::new(ui_family())
    }
}

pub fn text_light() -> iced::Font {
    iced::Font {
        weight: Weight::Light,
        ..iced::Font::new(ui_family())
    }
}

/// Monospace font for source view and code blocks.
/// Uses `Default::default()` intentionally — system monospace, not Inter.
pub fn monospace() -> iced::Font {
    iced::Font {
        family: iced::font::Family::Monospace,
        ..Default::default()
    }
}

// ── Sizes ────────────────────────────────────────────────

pub const TEXT_SIZE: f32 = 13.0;
pub const ICON_SIZE: f32 = 14.0;

// ── Line height ──────────────────────────────────────────

pub const DEFAULT_LINE_HEIGHT: f32 = 1.4;

pub fn line_height() -> LineHeight {
    LineHeight::Relative(DEFAULT_LINE_HEIGHT)
}

/// Resolve the absolute line height in pixels for a given font size.
pub fn resolve_line_height(font_size: f32) -> f32 {
    line_height().to_absolute(Pixels(font_size)).0
}

// ── Font loading ─────────────────────────────────────────

pub fn load() -> impl Iterator<Item = Cow<'static, [u8]>> {
    [
        Cow::Borrowed(include_bytes!("../fonts/InterVariable.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../fonts/InterVariable-Italic.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../fonts/lucide.ttf").as_slice()),
    ]
    .into_iter()
}
