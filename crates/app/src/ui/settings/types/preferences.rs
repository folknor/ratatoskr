use crate::db::DateDisplay;
use crate::pop_out::RenderingMode;

/// Controls the background color of email body containers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmailBodyBackground {
    /// Always use a white background (best for email rendering fidelity).
    #[default]
    AlwaysWhite,
    /// Use the current theme's background color.
    MatchTheme,
    /// White in light themes, theme background in dark themes.
    Auto,
}

impl EmailBodyBackground {
    pub fn label(self) -> &'static str {
        match self {
            Self::AlwaysWhite => "Always White",
            Self::MatchTheme => "Match Theme",
            Self::Auto => "Auto",
        }
    }

    pub fn from_label(s: &str) -> Self {
        match s {
            "Match Theme" => Self::MatchTheme,
            "Auto" => Self::Auto,
            _ => Self::AlwaysWhite,
        }
    }
}

/// Snapshot of user-facing preferences that support live preview.
/// Compared with `PartialEq` for change detection.
#[derive(Debug, Clone, PartialEq)]
pub struct PreferencesState {
    pub theme: String,
    pub scale: f32,
    pub density: String,
    pub font_size: String,
    pub date_display: DateDisplay,
    pub reading_pane_position: String,
    pub sync_status_bar: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
    pub default_rendering_mode: RenderingMode,
    pub email_body_background: EmailBodyBackground,
}
