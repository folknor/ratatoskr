use iced::Padding;

// ── Spacing scale (geometric progression) ───────────────
// Values: 0, 2, 4, 8, 12, 16, 24, 32, 48, 64
// Every padding/spacing in the app must use one of these.

/// 0px - No space
pub const SPACE_0: f32 = 0.0;
/// 2px - Hairline: badge padding, dot offsets
pub const SPACE_XXXS: f32 = 2.0;
/// 4px - Related elements: icon+text gap
pub const SPACE_XXS: f32 = 4.0;
/// 8px - Standard element gap, compact lists
pub const SPACE_XS: f32 = 8.0;
/// 12px - Comfortable list items, card sections
pub const SPACE_SM: f32 = 12.0;
/// 16px - Panel padding, section separators
pub const SPACE_MD: f32 = 16.0;
/// 24px - Major sections within a panel
pub const SPACE_LG: f32 = 24.0;
/// 32px - Panel margins, generous padding
pub const SPACE_XL: f32 = 32.0;
/// 48px - Large layout gaps
pub const SPACE_XXL: f32 = 48.0;
/// 64px - Page-level spacing
pub const SPACE_XXXL: f32 = 64.0;

// ── Type scale ─────────────────────────────────────────
// Every text .size() call must reference one of these.

/// 10px - Badges, section headers, tertiary metadata
pub const TEXT_XS: f32 = 10.0;
/// 11px - Snippets, timestamps, captions
pub const TEXT_SM: f32 = 11.0;
/// 12px - Body text, nav items, labels
pub const TEXT_MD: f32 = 12.0;
/// 13px - Emphasized body, compose button
pub const TEXT_LG: f32 = 13.0;
/// 14px - Icons at standard size
pub const TEXT_XL: f32 = 14.0;
/// 16px - Empty state titles, section titles
pub const TEXT_TITLE: f32 = 16.0;
/// 18px - Page titles, thread subject
pub const TEXT_HEADING: f32 = 18.0;

// ── Icon sizes ─────────────────────────────────────────
// Every icon .size() call must reference one of these.

/// 10px - Inline indicators (chevrons in section headers, attachment clips)
pub const ICON_XS: f32 = 10.0;
/// 11px - Compact UI icons (chevron in dropdown trigger)
pub const ICON_SM: f32 = 11.0;
/// 12px - Standard small icons (action bar, settings, nav)
pub const ICON_MD: f32 = 12.0;
/// 13px - Compose button icon
pub const ICON_LG: f32 = 13.0;
/// 14px - Dropdown items, reply buttons, leading slot icons
pub const ICON_XL: f32 = 14.0;
/// 28px - Mode switcher hero icon
pub const ICON_HERO: f32 = 28.0;
/// 24px - File type icon on attachment cards (near-doubled standard icon)
pub const ICON_ATTACHMENT_FILE: f32 = 24.0;
/// Fixed height of the file-info row inside an attachment card. Pinned so
/// the collapsed card is the same height regardless of whether the
/// "N versions" badge is present (the badge has button padding, the meta
/// line is bare text - without a fixed wrapper height the row jumps).
pub const ATTACHMENT_CARD_MAIN_ROW_HEIGHT: f32 = 36.0;
/// Fixed height of the bottom (meta) row inside an attachment card.
/// Matches the natural height of the versions badge (text size + button
/// padding) so the row keeps a consistent height whether or not the
/// badge is rendered. Without this, the filename row above shifts UP
/// vertically when the badge is present, because the centered text
/// column gets taller overall.
pub const ATTACHMENT_CARD_META_ROW_HEIGHT: f32 = TEXT_SM + 2.0 * 4.0;
/// Fixed height for the Open / Save action buttons that sit beside an
/// attachment card. Equals `ATTACHMENT_CARD_MAIN_ROW_HEIGHT` plus the
/// card's vertical padding (PAD_NAV_ITEM top + bottom = 8) so the buttons
/// exactly span the collapsed card's height.
pub const ATTACHMENT_ACTION_BTN_HEIGHT: f32 = ATTACHMENT_CARD_MAIN_ROW_HEIGHT + 8.0;
/// Fixed width for the "Attachments" group header's right-side action
/// button (Save All). Has to equal the file row's [Open, Save] cluster
/// width (`2 * ATTACHMENT_ICON_BTN_WIDTH + SPACE_XS`) so the header's
/// title area ends where the bordered file card ends.
pub const ATTACHMENT_HEADER_ACTION_WIDTH: f32 =
    2.0 * ATTACHMENT_ICON_BTN_WIDTH + SPACE_XS;
/// Fixed width for each Open / Save icon button on a file row. Together
/// with `SPACE_XS` between them and `ATTACHMENT_HEADER_ACTION_WIDTH` on
/// Save All, this is what aligns the right edges of the title area and
/// the bordered file card. Bumped from the natural ~28px so the icons
/// have visual breathing room.
pub const ATTACHMENT_ICON_BTN_WIDTH: f32 = 36.0;
/// Height of a compact attachment pill in the pop-out message viewer.
/// Single-row content (`[icon] filename size`), shorter than the reading
/// pane card since there is no meta line or version list.
pub const POPOUT_ATTACHMENT_CARD_HEIGHT: f32 = 36.0;
/// Visible inner-scroll height of the pinned attachments panel in the
/// pop-out message viewer. Sized to expose ~1.8 rows of cards so the
/// scrollbar's existence is self-evident.
pub const POPOUT_ATTACHMENT_PANEL_INNER_HEIGHT: f32 =
    POPOUT_ATTACHMENT_CARD_HEIGHT * 1.8 + SPACE_XS;

// ── Avatar sizes ───────────────────────────────────────
// Every avatar_circle() call must reference one of these.

/// 20px - Dropdown menu items
pub const AVATAR_DROPDOWN_ITEM: f32 = 20.0;
/// 24px - Dropdown trigger, compact inline
pub const AVATAR_DROPDOWN_TRIGGER: f32 = 24.0;
/// 28px - Thread list cards
pub const AVATAR_THREAD_CARD: f32 = 28.0;
/// 32px - Message cards in reading pane
pub const AVATAR_MESSAGE_CARD: f32 = 32.0;
/// 56px - Contact sidebar hero
pub const AVATAR_CONTACT_HERO: f32 = 56.0;

/// 14px - Radio button circle in dropdown rows / picker rows. Outer ring
/// at this size, inner filled disk at ~50% of this size when selected.
pub const RADIO_CIRCLE_SIZE: f32 = 14.0;

// ── Leading slot widths ────────────────────────────────
// When a list item has an icon or avatar on the left,
// wrap it in a fixed-width container using these values
// so all labels in the list align vertically.

/// Icon/avatar slot width for all dropdown rows (trigger and items).
/// Must be >= the largest avatar used in dropdowns.
pub const SLOT_DROPDOWN: f32 = AVATAR_DROPDOWN_TRIGGER;

// ── Color dot ──────────────────────────────────────────

/// Label color dot diameter
pub const DOT_SIZE: f32 = 8.0;

// ── Border radii ───────────────────────────────────────
// Every border::rounded() or radius value must use one of these.

/// 4px - Buttons, nav items, input fields
pub const RADIUS_SM: f32 = 4.0;
/// 6px - Elevated containers
pub const RADIUS_MD: f32 = 6.0;
/// 8px - Cards, floating menus, badges, primary button
pub const RADIUS_LG: f32 = 8.0;

// ── Item heights ────────────────────────────────────────

pub const DROPDOWN_ITEM_HEIGHT: f32 = 32.0;
pub const DROPDOWN_TRIGGER_HEIGHT: f32 = 40.0;

// ── Panel widths ────────────────────────────────────────

pub const SIDEBAR_WIDTH: f32 = 180.0;
pub const THREAD_LIST_WIDTH: f32 = 400.0;
pub const RIGHT_SIDEBAR_WIDTH: f32 = 240.0;

/// Label dot diameter on thread cards (line 3 indicators)
pub const LABEL_DOT_SIZE: f32 = 6.0;

/// Thread card fixed height (three lines + padding)
pub const THREAD_CARD_HEIGHT: f32 = 68.0;

/// Right sidebar section header padding
pub const PAD_RIGHT_SIDEBAR: Padding = Padding {
    top: 12.0,
    right: 12.0,
    bottom: 12.0,
    left: 12.0,
};

/// Starred thread card warm background alpha
pub const STARRED_BG_ALPHA: f32 = 0.25;

/// Auto-collapse right sidebar when window width drops below this
pub const RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH: f32 = 1200.0;

// ── Per-pane minimum widths (for resize clamping) ──────

pub const SIDEBAR_MIN_WIDTH: f32 = 220.0;
/// Height of the sidebar header (mode button + dropdown + compose stack)
pub const SIDEBAR_HEADER_HEIGHT: f32 = 76.0;
pub const THREAD_LIST_MIN_WIDTH: f32 = 250.0;
pub const READING_PANE_MIN_WIDTH: f32 = 300.0;

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

/// Collapsible section header (tighter horizontal padding).
pub const PAD_COLLAPSIBLE_HEADER: Padding = Padding {
    top: 0.0,
    right: 8.0,
    bottom: 0.0,
    left: 8.0,
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

/// Dropdown: trigger button and menu wrapper.
pub const PAD_DROPDOWN: Padding = Padding::new(8.0);

/// Settings content area (generous margins around fieldsets).
pub const PAD_SETTINGS_CONTENT: Padding = Padding {
    top: 32.0,
    right: 48.0,
    bottom: 32.0,
    left: 48.0,
};

/// Minimum height for settings rows (ensures consistent row height
/// whether the row contains a control or just text).
pub const SETTINGS_ROW_HEIGHT: f32 = 52.0;
/// Toggle rows with label + description need more room.
pub const SETTINGS_TOGGLE_ROW_HEIGHT: f32 = 64.0;

/// Settings row (label + control).
pub const PAD_SETTINGS_ROW: Padding = Padding {
    top: 12.0,
    right: 16.0,
    bottom: 12.0,
    left: 16.0,
};

/// Select trigger padding (vertical only - sits inside a padded row).
pub const PAD_SELECT_TRIGGER: Padding = Padding {
    top: 4.0,
    right: 0.0,
    bottom: 4.0,
    left: 0.0,
};

/// Message body inner padding.
pub const PAD_BODY: Padding = Padding {
    top: 12.0,
    right: 16.0,
    bottom: 12.0,
    left: 16.0,
};

// ── Settings-specific sizes ────────────────────────────

/// Settings nav sidebar width
pub const SETTINGS_NAV_WIDTH: f32 = 200.0;

/// Settings content max width
pub const SETTINGS_CONTENT_MAX_WIDTH: u32 = 600;

/// Width for editor action buttons (Save, Cancel, Delete).
pub const EDITOR_BUTTON_WIDTH: f32 = 100.0;

/// Minimum width for select widget (trigger + menu)
pub const SELECT_MIN_WIDTH: f32 = 140.0;

/// Help tooltip popup width
pub const HELP_TOOLTIP_WIDTH: f32 = 300.0;

/// Slider handle radius
pub const SLIDER_HANDLE_RADIUS: f32 = 7.0;
/// Slider rail width (height of the track)
pub const SLIDER_RAIL_WIDTH: f32 = 4.0;

/// Radio button outer circle size
pub const RADIO_SIZE: f32 = 16.0;
/// Spacing between radio circle and label
pub const RADIO_LABEL_SPACING: f32 = SPACE_SM;

/// Width of the grip handle slot in editable lists
pub const GRIP_SLOT_WIDTH: f32 = 16.0;

/// Space between scrollbar and content (embeds scrollbar instead of overlaying)
pub const SCROLLBAR_SPACING: f32 = SPACE_XXXS;

// ── Palette overlay ─────────────────────────────────────

/// Search typeahead popup max height.
pub const TYPEAHEAD_MAX_HEIGHT: f32 = 300.0;

/// Palette card width.
pub const PALETTE_WIDTH: f32 = 600.0;
/// Palette card max height.
pub const PALETTE_MAX_HEIGHT: f32 = 400.0;
/// Palette vertical offset from top of window.
pub const PALETTE_TOP_OFFSET: f32 = 80.0;
/// Individual result row height.
pub const PALETTE_RESULT_HEIGHT: f32 = 36.0;
/// Category badge column width.
pub const PALETTE_CATEGORY_WIDTH: f32 = 80.0;
/// Keybinding hint column width.
pub const PALETTE_KEYBINDING_WIDTH: f32 = 100.0;

// ── Status bar ──────────────────────────────────────────

// ── Account modal ───────────────────────────────────────

/// Add Account modal width
pub const ACCOUNT_MODAL_WIDTH: f32 = 520.0;
/// Add Account modal max height
pub const ACCOUNT_MODAL_MAX_HEIGHT: f32 = 640.0;
/// Color swatch size in the palette picker
pub const COLOR_SWATCH_SIZE: f32 = 28.0;
/// Color palette grid columns
pub const COLOR_PALETTE_COLUMNS: usize = 5;
/// Protocol selection card height
pub const PROTOCOL_CARD_HEIGHT: f32 = 64.0;
/// Welcome icon size on the first-launch email input screen.
pub const WELCOME_ICON_SIZE: f32 = 96.0;
/// Color swatch button padding (around the canvas).
pub const PAD_COLOR_SWATCH: f32 = 2.0;
/// Alpha for dimmed (already-used) color swatches.
pub const COLOR_SWATCH_DIMMED_ALPHA: f32 = 0.35;
/// Proportional factor for check-mark size relative to swatch radius.
pub const COLOR_SWATCH_CHECK_SCALE: f32 = 0.35;

/// Status bar fixed height (one line of text + padding).
pub const STATUS_BAR_HEIGHT: f32 = 28.0;

/// Status bar internal padding (compact vertical, standard horizontal).
pub const PAD_STATUS_BAR: Padding = Padding {
    top: 4.0,
    right: 12.0,
    bottom: 4.0,
    left: 12.0,
};

// ── Token input ─────────────────────────────────────────

/// Token chip height.
pub const TOKEN_HEIGHT: f32 = 24.0;
/// Token chip border radius.
pub const TOKEN_RADIUS: f32 = RADIUS_SM;
/// Token chip internal padding.
pub const PAD_TOKEN: Padding = Padding {
    top: 2.0,
    right: 8.0,
    bottom: 2.0,
    left: 8.0,
};
/// Spacing between tokens (horizontal).
pub const TOKEN_SPACING: f32 = SPACE_XXS;
/// Spacing between token rows (vertical).
pub const TOKEN_ROW_SPACING: f32 = SPACE_XXS;
/// Token input field internal padding. Horizontal sides match `PAD_INPUT`
/// so the placeholder/text align with iced's default `text_input` (e.g.
/// the Subject field next to To/Cc/Bcc in the compose window).
pub const PAD_TOKEN_INPUT: Padding = Padding {
    top: 4.0,
    right: 12.0,
    bottom: 4.0,
    left: 12.0,
};
/// Minimum width for the text input portion before wrapping.
pub const TOKEN_TEXT_MIN_WIDTH: f32 = 120.0;
/// Average character width as a proportion of `TEXT_MD` for layout estimation.
/// Used in token chip width calculation and cursor positioning.
pub const TOKEN_AVG_CHAR_WIDTH_FACTOR: f32 = 0.54;

// ── Calendar ────────────────────────────────────────────

/// Minimum height for a day cell in the month grid.
pub const CALENDAR_CELL_MIN_HEIGHT: f32 = 80.0;
/// Height of a single event entry row in the month grid.
pub const CALENDAR_EVENT_HEIGHT: f32 = 20.0;
/// Height of the day-of-week header row in the month grid.
/// Calendar sidebar header height (matches mail sidebar dropdown + gap + compose)
pub const CALENDAR_HEADER_HEIGHT: f32 = 64.0;
/// Cell size (width and height) for the mini-month date grid.
pub const MINI_MONTH_CELL_SIZE: f32 = 28.0;

// ── Time grid (day/work-week/week views) ─────────────

/// Width of the hour-label column on the left of the time grid.
pub const TIME_GRID_HOUR_LABEL_WIDTH: f32 = 60.0;
/// Vertical pixels per hour in the time grid.
pub const TIME_GRID_PIXELS_PER_HOUR: f32 = 60.0;
/// Minimum rendered height for very short events.
pub const TIME_GRID_MIN_EVENT_HEIGHT: f32 = 20.0;
/// Height of each day-column header (date + weekday label).
pub const TIME_GRID_HEADER_HEIGHT: f32 = 48.0;
/// Height of the all-day event bar above the time grid.
pub const TIME_GRID_ALL_DAY_HEIGHT: f32 = 32.0;
/// Thickness of the current-time indicator line.
pub const TIME_GRID_NOW_LINE_WIDTH: f32 = 2.0;

// ── Calendar overlay (event detail / editor) ────────────
/// Width of the event detail/editor modal.
pub const CALENDAR_OVERLAY_WIDTH: f32 = 420.0;
/// Maximum height of the event detail/editor modal.
pub const CALENDAR_OVERLAY_MAX_HEIGHT: f32 = 560.0;
/// Height of a form row in the event editor.
pub const CALENDAR_FORM_ROW_HEIGHT: f32 = 36.0;
/// Width of the label column in the event editor form.
pub const CALENDAR_FORM_LABEL_WIDTH: f32 = 80.0;
/// Group icon size on group tokens.
pub const TOKEN_GROUP_ICON_SIZE: f32 = ICON_XS;

// ── Autocomplete dropdown ───────────────────────────────

/// Maximum height of the autocomplete dropdown.
pub const AUTOCOMPLETE_MAX_HEIGHT: f32 = 300.0;
/// Height of each autocomplete suggestion row.
pub const AUTOCOMPLETE_ROW_HEIGHT: f32 = 32.0;

// ── Message view pop-out window ─────────────────────────
pub const MESSAGE_VIEW_DEFAULT_WIDTH: f32 = 800.0;
pub const MESSAGE_VIEW_DEFAULT_HEIGHT: f32 = 600.0;
pub const MESSAGE_VIEW_MIN_WIDTH: f32 = 480.0;
pub const MESSAGE_VIEW_MIN_HEIGHT: f32 = 320.0;

// ── Compose pop-out window ─────────────────────────────
pub const COMPOSE_DEFAULT_WIDTH: f32 = 720.0;
pub const COMPOSE_DEFAULT_HEIGHT: f32 = 560.0;
pub const COMPOSE_MIN_WIDTH: f32 = 480.0;
pub const COMPOSE_MIN_HEIGHT: f32 = 360.0;
/// Width of the label column (From, To, Cc, Bcc, Subject) in compose.
pub const COMPOSE_LABEL_WIDTH: f32 = 52.0;

// ── Emoji picker ────────────────────────────────────────

/// Emoji picker popup width.
pub const EMOJI_PICKER_WIDTH: f32 = 300.0;
/// Emoji picker popup max height.
pub const EMOJI_PICKER_MAX_HEIGHT: f32 = 350.0;
/// Individual emoji button size (width and height).
pub const EMOJI_BUTTON_SIZE: f32 = 36.0;
/// Number of columns in the emoji grid.
pub const EMOJI_GRID_COLUMNS: usize = 8;
/// Font size for emoji glyphs in the picker grid.
pub const EMOJI_FONT_SIZE: f32 = 20.0;

// ── Chat timeline ───────────────────────────────────────
/// Maximum width for a chat bubble (prevents full-width stretch).
pub const CHAT_BUBBLE_MAX_WIDTH: f32 = 480.0;
/// Border radius for chat bubbles (larger than card radius).
pub const CHAT_BUBBLE_RADIUS: f32 = 12.0;
/// Internal padding for chat bubble content.
pub const PAD_CHAT_BUBBLE: Padding = Padding {
    top: 8.0,
    right: 12.0,
    bottom: 8.0,
    left: 12.0,
};
/// Spacing between consecutive same-sender bubbles.
pub const CHAT_BUBBLE_SPACING: f32 = 4.0;
/// Spacing between sender changes (larger gap).
pub const CHAT_GROUP_SPACING: f32 = 12.0;
/// Spacing around date separator lines.
pub const CHAT_DATE_SEPARATOR_SPACING: f32 = 16.0;

// ── People tab list panel ──────────────────────────────
/// Fixed height of the recessed scrollable panel holding contact pills
/// in the People tab. Targets ~5 minimal cards visible at a glance.
pub const CONTACT_PANEL_HEIGHT: f32 = 380.0;
/// Same idea for the Groups list panel.
pub const GROUP_PANEL_HEIGHT: f32 = 240.0;
/// Vertical spacing between contact / group pills inside their panel.
pub const PEOPLE_PILL_SPACING: f32 = 8.0;
