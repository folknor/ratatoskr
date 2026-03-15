use iced::Padding;

// ── Spacing scale (geometric progression) ───────────────
// Values: 0, 2, 4, 8, 12, 16, 24, 32, 48, 64
// Every padding/spacing in the app must use one of these.

/// 0px — No space
pub const SPACE_0: f32 = 0.0;
/// 2px — Hairline: badge padding, dot offsets
pub const SPACE_XXXS: f32 = 2.0;
/// 4px — Related elements: icon+text gap
pub const SPACE_XXS: f32 = 4.0;
/// 8px — Standard element gap, compact lists
pub const SPACE_XS: f32 = 8.0;
/// 12px — Comfortable list items, card sections
pub const SPACE_SM: f32 = 12.0;
/// 16px — Panel padding, section separators
pub const SPACE_MD: f32 = 16.0;
/// 24px — Major sections within a panel
pub const SPACE_LG: f32 = 24.0;
/// 32px — Panel margins, generous padding
pub const SPACE_XL: f32 = 32.0;
/// 48px — Large layout gaps
pub const SPACE_XXL: f32 = 48.0;
/// 64px — Page-level spacing
pub const SPACE_XXXL: f32 = 64.0;

// ── Panel widths ────────────────────────────────────────

pub const SIDEBAR_WIDTH: f32 = 180.0;
pub const THREAD_LIST_WIDTH: f32 = 280.0;
pub const CONTACT_SIDEBAR_WIDTH: f32 = 240.0;

// ── Padding presets ─────────────────────────────────────
// Named by usage, not by raw values. All values land on
// the spacing scale above.

/// Compact inline element: icon buttons, badges, tags.
pub const PAD_ICON_BTN: Padding = Padding {
    top: 4.0,
    right: 8.0,
    bottom: 4.0,
    left: 8.0,
};

/// Nav / sidebar item.
pub const PAD_NAV_ITEM: Padding = Padding {
    top: 4.0,
    right: 8.0,
    bottom: 4.0,
    left: 8.0,
};

/// Standard button (e.g. compose, reply).
pub const PAD_BUTTON: Padding = Padding {
    top: 8.0,
    right: 16.0,
    bottom: 8.0,
    left: 16.0,
};

/// Sidebar wrapper.
pub const PAD_SIDEBAR: Padding = Padding {
    top: 8.0,
    right: 4.0,
    bottom: 8.0,
    left: 4.0,
};

/// Panel header area (search bar, title row).
pub const PAD_PANEL_HEADER: Padding = Padding {
    top: 12.0,
    right: 12.0,
    bottom: 12.0,
    left: 12.0,
};

/// Toolbar / action bar.
pub const PAD_TOOLBAR: Padding = Padding {
    top: 8.0,
    right: 16.0,
    bottom: 8.0,
    left: 16.0,
};

/// Content section (thread header, message list margins).
pub const PAD_CONTENT: Padding = Padding {
    top: 16.0,
    right: 24.0,
    bottom: 16.0,
    left: 24.0,
};

/// Card internal padding (message cards, elevated panels).
pub const PAD_CARD: Padding = Padding {
    top: 12.0,
    right: 16.0,
    bottom: 12.0,
    left: 16.0,
};

/// Thread card internal content.
pub const PAD_THREAD_CARD: Padding = Padding {
    top: 8.0,
    right: 12.0,
    bottom: 8.0,
    left: 12.0,
};

/// Search / text input field.
pub const PAD_INPUT: Padding = Padding {
    top: 8.0,
    right: 12.0,
    bottom: 8.0,
    left: 12.0,
};

/// Section header in a sidebar or panel.
pub const PAD_SECTION_HEADER: Padding = Padding {
    top: 8.0,
    right: 12.0,
    bottom: 8.0,
    left: 12.0,
};

/// Stat row / key-value pair.
pub const PAD_STAT_ROW: Padding = Padding {
    top: 2.0,
    right: 12.0,
    bottom: 2.0,
    left: 12.0,
};

/// Badge / count pill.
pub const PAD_BADGE: Padding = Padding {
    top: 2.0,
    right: 4.0,
    bottom: 2.0,
    left: 4.0,
};

/// Account switcher button.
pub const PAD_ACCOUNT: Padding = Padding::new(8.0);
