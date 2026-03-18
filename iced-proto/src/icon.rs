use iced::widget::text::{LineHeight, Text};

pub const FONT: iced::Font = iced::Font::new("lucide");
pub const SIZE: f32 = 14.0;

/// Build an icon Text from a codepoint. Used by widgets that accept
/// icon codepoints as data instead of pre-built Text elements.
pub fn to_icon<'a>(unicode: char) -> Text<'a> {
    iced::widget::text(unicode.to_string())
        .line_height(LineHeight::Relative(1.0))
        .size(SIZE)
        .font(FONT)
}

fn to_text<'a>(unicode: char) -> Text<'a> { to_icon(unicode) }

// ── Codepoints (for data-driven widgets) ────────────────
pub const INBOX_CODEPOINT: char = '\u{e0f7}';

// ── Email actions ────────────────────────────────────────
pub fn mail<'a>() -> Text<'a> { to_text('\u{e10f}') }
pub fn inbox<'a>() -> Text<'a> { to_text(INBOX_CODEPOINT) }
pub fn send<'a>() -> Text<'a> { to_text('\u{e152}') }
pub fn reply<'a>() -> Text<'a> { to_text('\u{e22a}') }
pub fn reply_all<'a>() -> Text<'a> { to_text('\u{e22b}') }
pub fn forward<'a>() -> Text<'a> { to_text('\u{e229}') }
pub fn archive<'a>() -> Text<'a> { to_text('\u{e041}') }
pub fn trash<'a>() -> Text<'a> { to_text('\u{e18e}') }
pub fn star<'a>() -> Text<'a> { to_text('\u{e176}') }
pub fn pin<'a>() -> Text<'a> { to_text('\u{e259}') }
pub fn paperclip<'a>() -> Text<'a> { to_text('\u{e12d}') }

// ── Navigation ───────────────────────────────────────────
pub fn search<'a>() -> Text<'a> { to_text('\u{e151}') }
pub fn settings<'a>() -> Text<'a> { to_text('\u{e154}') }
pub fn pencil<'a>() -> Text<'a> { to_text('\u{e1f9}') }
pub fn edit<'a>() -> Text<'a> { to_text('\u{e172}') }
pub fn menu<'a>() -> Text<'a> { to_text('\u{e115}') }
pub fn filter<'a>() -> Text<'a> { to_text('\u{e0dc}') }

// ── Indicators ───────────────────────────────────────────
pub fn check<'a>() -> Text<'a> { to_text('\u{e06c}') }
pub fn x<'a>() -> Text<'a> { to_text('\u{e1b2}') }
pub fn plus<'a>() -> Text<'a> { to_text('\u{e13d}') }
pub fn minus<'a>() -> Text<'a> { to_text('\u{e11c}') }
pub fn info<'a>() -> Text<'a> { to_text('\u{e0f9}') }
pub fn alert_triangle<'a>() -> Text<'a> { to_text('\u{e193}') }
pub fn help_circle<'a>() -> Text<'a> { to_text('\u{e082}') }

// ── Chevrons & arrows ────────────────────────────────────
pub fn chevron_down<'a>() -> Text<'a> { to_text('\u{e06d}') }
pub fn chevron_left<'a>() -> Text<'a> { to_text('\u{e06e}') }
pub fn chevron_right<'a>() -> Text<'a> { to_text('\u{e06f}') }
pub fn arrow_left<'a>() -> Text<'a> { to_text('\u{e048}') }
pub fn arrow_right<'a>() -> Text<'a> { to_text('\u{e049}') }
pub fn arrow_up<'a>() -> Text<'a> { to_text('\u{e04a}') }
pub fn arrow_down<'a>() -> Text<'a> { to_text('\u{e042}') }

// ── Content ──────────────────────────────────────────────
pub fn eye<'a>() -> Text<'a> { to_text('\u{e0ba}') }
pub fn eye_off<'a>() -> Text<'a> { to_text('\u{e0bb}') }
pub fn copy<'a>() -> Text<'a> { to_text('\u{e09e}') }
pub fn scissors<'a>() -> Text<'a> { to_text('\u{e14e}') }
pub fn link<'a>() -> Text<'a> { to_text('\u{e102}') }
pub fn external_link<'a>() -> Text<'a> { to_text('\u{e0b9}') }
pub fn image<'a>() -> Text<'a> { to_text('\u{e0f6}') }
pub fn file<'a>() -> Text<'a> { to_text('\u{e0c0}') }
pub fn file_text<'a>() -> Text<'a> { to_text('\u{e0cc}') }
pub fn printer<'a>() -> Text<'a> { to_text('\u{e141}') }
pub fn download<'a>() -> Text<'a> { to_text('\u{e0b2}') }
pub fn upload<'a>() -> Text<'a> { to_text('\u{e19e}') }

// ── Formatting ───────────────────────────────────────────
pub fn bold<'a>() -> Text<'a> { to_text('\u{e05d}') }
pub fn italic<'a>() -> Text<'a> { to_text('\u{e0fb}') }
pub fn underline<'a>() -> Text<'a> { to_text('\u{e19a}') }
pub fn list<'a>() -> Text<'a> { to_text('\u{e106}') }
pub fn align_left<'a>() -> Text<'a> { to_text('\u{e185}') }

// ── People & categories ──────────────────────────────────
pub fn user<'a>() -> Text<'a> { to_text('\u{e19f}') }
pub fn users<'a>() -> Text<'a> { to_text('\u{e1a4}') }
pub fn tag<'a>() -> Text<'a> { to_text('\u{e17f}') }
pub fn folder<'a>() -> Text<'a> { to_text('\u{e0d7}') }
pub fn bookmark<'a>() -> Text<'a> { to_text('\u{e060}') }
pub fn flag<'a>() -> Text<'a> { to_text('\u{e0d1}') }
pub fn hash<'a>() -> Text<'a> { to_text('\u{e0ef}') }
pub fn at_sign<'a>() -> Text<'a> { to_text('\u{e04e}') }
pub fn smile<'a>() -> Text<'a> { to_text('\u{e164}') }

// ── Time & notifications ─────────────────────────────────
pub fn bell<'a>() -> Text<'a> { to_text('\u{e059}') }
pub fn calendar<'a>() -> Text<'a> { to_text('\u{e063}') }
pub fn clock<'a>() -> Text<'a> { to_text('\u{e087}') }

// ── Drag & layout ───────────────────────────────────────
pub fn grip_vertical<'a>() -> Text<'a> { to_text('\u{e0eb}') }

// ── Actions ──────────────────────────────────────────────
pub fn undo<'a>() -> Text<'a> { to_text('\u{e19b}') }
pub fn redo<'a>() -> Text<'a> { to_text('\u{e143}') }
pub fn refresh<'a>() -> Text<'a> { to_text('\u{e145}') }
pub fn ellipsis<'a>() -> Text<'a> { to_text('\u{e0b6}') }
pub fn ellipsis_vertical<'a>() -> Text<'a> { to_text('\u{e0b7}') }

// ── Appearance ──────────────────────────────────────────
pub fn palette<'a>() -> Text<'a> { to_text('\u{e1dd}') }

// ── System ───────────────────────────────────────────────
pub fn moon<'a>() -> Text<'a> { to_text('\u{e11e}') }
pub fn sun<'a>() -> Text<'a> { to_text('\u{e178}') }
pub fn monitor<'a>() -> Text<'a> { to_text('\u{e11d}') }
pub fn globe<'a>() -> Text<'a> { to_text('\u{e0e8}') }
pub fn lock<'a>() -> Text<'a> { to_text('\u{e10b}') }
pub fn unlock<'a>() -> Text<'a> { to_text('\u{e10c}') }
pub fn shield<'a>() -> Text<'a> { to_text('\u{e158}') }
pub fn zap<'a>() -> Text<'a> { to_text('\u{e1b4}') }
