# iced-proto

Prototype iced UI for the Ratatoskr email client. Renders a four-pane email interface using iced 0.15-dev (Halloy's fork) against a seeded test database.

## Commands

- `python3 seed-db.py [thunderbird.sqlite] [output-dir]` — seed a test DB from Thunderbird's `global-messages-db.sqlite` (defaults: `./thunderbird.sqlite` → `~/.local/share/com.velo.app/ratatoskr.db`)
- `cargo run` — run the prototype (requires a seeded `ratatoskr.db` in `~/.local/share/com.velo.app/`)
- `cargo check` — type-check
- `cargo clippy` — lint

## Commits

Don't create standalone commits for documentation-only changes (CLAUDE.md, TODO.md, comments). Fold them into the next real code commit instead. Keep the git history focused on meaningful changes.

## Architecture

Elm architecture (iced's `application()` — boot/update/view cycle). Single `App` struct holds all state. All DB access is async via `tokio::task::spawn_blocking` through a shared `Arc<Mutex<Connection>>`.

### Files

- **`seed-db.py`** — Creates a Ratatoskr-schema DB from Thunderbird's `global-messages-db.sqlite`. Extracts accounts from IMAP folder URIs, groups messages into threads by Thunderbird's `conversationID`, parses sender/recipient fields, and populates accounts/labels/threads/messages/contacts tables. Does NOT copy email bodies (just metadata + subjects as snippets). A `thunderbird.sqlite` file is checked into the repo for convenience.

### Layout

```
[ Sidebar 180px | Thread List 280px | Reading Pane (fill) | Contact Sidebar 240px ]
```

### Message flow

1. `boot()` → loads accounts from DB
2. `AccountsLoaded` → auto-selects first account, fires parallel loads for labels + threads
3. `SelectAccount` / `SelectLabel` → reloads threads for new filter
4. `SelectThread` → updates reading pane + contact sidebar

### What's real vs placeholder

**Real:** Account loading, label listing, thread queries with label filtering, date formatting (relative: time/day/date), read/unread styling, avatar colors — all data from the seeded Thunderbird-sourced DB using the real Ratatoskr schema.

**Placeholder:** Reading pane shows snippet instead of actual message body (no body store access yet), contact sidebar stats are hardcoded dashes, Compose/Settings/action buttons emit `Noop`, search bar is non-functional.

## Gotchas

**`Padding::from` with mixed types:** `Padding::from([0, CONSTANT])` won't compile if `CONSTANT` is `f32` — Rust infers the array as `[i32; 2]`. Always use `[0.0, CONSTANT]` to keep both elements `f32`.

**`iced::Font::DEFAULT` is not Inter:** We set `default_font(font::TEXT)` which is `Font::with_name("Inter")`. If you construct a font with `iced::Font { weight, ..iced::Font::DEFAULT }` it will NOT use Inter. Always spread from `font::TEXT` instead: `iced::Font { weight, ..font::TEXT }`.

**Button `text_color` doesn't reach children with explicit `.style()`.** If you set `text_color` on a button style but the `text()` or icon inside has its own `.style(some_fn)`, the explicit style wins. The button's `text_color` only affects children that don't override it. This means changing a button style's color has no visible effect when all children set their own style — you have to change the text/icon styles too.

**Popover menu width is constrained to the trigger's width.** The `PopoverOverlay` layout uses `base_bounds.width` as the menu's max width. If the trigger is `Length::Shrink`, the menu will be tiny. For narrow triggers (like the `select` widget), set an explicit width on the trigger (e.g., `SELECT_MIN_WIDTH`) so the menu has room.

**`height()` on containers is fixed, not minimum.** There's no `min_height()` on containers in this iced version. If you need rows to be "at least X tall but grow for bigger content," use a shared `height()` constant and accept that all rows are that exact height. Use different constants for different row types (e.g., `SETTINGS_ROW_HEIGHT` vs `SETTINGS_TOGGLE_ROW_HEIGHT`).

**Extended palette background scale (dark mode).** `base` is the darkest. Each step lightens by a fixed deviation: `base` (0%) → `weakest` (3%) → `weaker` (7%) → `weak` (10%) → `neutral` (12.5%) → `strong` (15%) → `stronger` (17.5%) → `strongest` (20%). In light mode the direction reverses. Use this to create visual depth hierarchy — e.g., fieldsets at `base`, content area at `weakest`, sidebar at `weaker`.

## Layout module (`src/ui/layout.rs`)

All sizing, spacing, padding, and radii are centralized here. Views import `use crate::ui::layout::*` and reference named constants. **No magic numbers in view or widget code** — every `.size()`, avatar diameter, border radius, and padding must reference a layout constant.

**Spacing scale** (geometric): `SPACE_XXXS` (2) → `SPACE_XXS` (4) → `SPACE_XS` (8) → `SPACE_SM` (12) → `SPACE_MD` (16) → `SPACE_LG` (24) → `SPACE_XL` (32) → `SPACE_XXL` (48) → `SPACE_XXXL` (64).

**Type scale:** `TEXT_XS` (10) → `TEXT_SM` (11) → `TEXT_MD` (12) → `TEXT_LG` (13) → `TEXT_XL` (14) → `TEXT_TITLE` (16) → `TEXT_HEADING` (18). Every `text(...).size()` must use one of these.

**Icon sizes:** `ICON_XS` (10) → `ICON_SM` (11) → `ICON_MD` (12) → `ICON_LG` (13) → `ICON_XL` (14). Every `icon::foo().size()` must use one of these.

**Avatar sizes:** `AVATAR_DROPDOWN_ITEM` (20), `AVATAR_DROPDOWN_TRIGGER` (24), `AVATAR_THREAD_CARD` (28), `AVATAR_MESSAGE_CARD` (32), `AVATAR_CONTACT_HERO` (56). Every `avatar_circle()` call must use one of these.

**Leading slot widths:** `SLOT_DROPDOWN`. When a list item has an icon or avatar on the left, wrap it in a fixed-size container so all labels align.

**Border radii:** `RADIUS_SM` (4), `RADIUS_MD` (6), `RADIUS_LG` (8). Every `border::rounded()` or `radius:` value must use one of these.

**Padding presets** are named by role: `PAD_ICON_BTN`, `PAD_NAV_ITEM`, `PAD_BUTTON`, `PAD_SIDEBAR`, `PAD_PANEL_HEADER`, `PAD_TOOLBAR`, `PAD_CONTENT`, `PAD_CARD`, `PAD_THREAD_CARD`, `PAD_INPUT`, `PAD_SECTION_HEADER`, `PAD_COLLAPSIBLE_HEADER`, `PAD_STAT_ROW`, `PAD_BADGE`, `PAD_DROPDOWN`, `PAD_BODY`.

**Panel widths:** `SIDEBAR_WIDTH` (180), `THREAD_LIST_WIDTH` (280), `CONTACT_SIDEBAR_WIDTH` (240).

**Semantic colors** live in `theme.rs`: `theme::ON_AVATAR` (white text/icons on colored backgrounds). No `Color::WHITE` or other raw colors in view code.

## Widget design rules

### A widget owns its entire layout.

The widget function builds every container, row, spacing, and style internally. There is no "partial widget" that the caller finishes assembling. If two instances of a widget should look the same, they will — because there is exactly one code path.

### Widget constructors accept data, not UI elements.

Constructors take primitive values (`&str`, `bool`, `usize`) and domain objects (`&Account`, `&Thread`). They never accept `Element`, `Row`, `Container`, or anything the caller built from iced primitives. The widget reads data from what it's given and builds all the UI internally.

### All widgets belong in widgets.rs.

Domain-specific widgets do not exist unless the user asks for it explicitly. A "scope dropdown" is not a widget — it's a sidebar view function that calls the generic `dropdown` widget with account data. The generic `dropdown` widget lives in `widgets.rs`. The sidebar-specific assembly lives in `sidebar.rs`.

### Every slot in a structured widget gets its own container.

In iced, bare widgets (especially `Text`) behave differently from widgets inside containers. A `text()` in a `row![]` negotiates its own width based on content. A `container(text())` with explicit constraints is predictable.

Name the slots and give each one a container:
- `icon_slot`: `container(icon).width(FIXED).height(FIXED).align_x(Center).align_y(Center)`
- `label_slot`: `container(text).width(Fill).align_y(Center)`

### `center()` vs `align_x/y(Center)` — know the difference.

- `center(Length::Fill)` — the content expands to fill the container. For text widgets, this stretches them to the container width.
- `center(Length::Shrink)` — the content stays at natural size, centered within the container.
- `align_x(Alignment::Center)` + `align_y(Alignment::Center)` — centers without giving the content a size hint. Safest for mixed content types.

Default to `align_x/y(Center)` for icon slots. Only use `center(Length::Fill)` when you specifically need the content to expand (e.g., centering a letter inside an avatar stack).

### Style functions override button text_color.

When a row has icon + text slots inside a button, don't rely on the button style's `text_color` to control their color. Each slot with an explicit `.style()` call must use the correct style function directly. If you add a `text_muted` style for inactive nav buttons, both the icon and label need to reference it — the button style alone won't propagate.

### Don't guess at visual issues.

When the user reports a visual bug and the fix isn't obvious from reading the code:
1. Understand the widget tree structure first (what contains what).
2. Reason about how each container constrains its children.
3. If uncertain how an iced API works, check reference projects (halloy, libcosmic) rather than guessing.
4. Ask for a description or screenshot before attempting a fix. Don't iterate blindly.

## Theme system (`src/ui/theme/`)

Custom `Theme` enum replacing iced's built-in, with `iced::theme::Base` impl. Per-widget `Catalog` implementations: button, checkbox, container, menu, pane_grid, progress_bar, rule, scrollable, text, text_editor, text_input. Each has named public style functions (e.g. `theme::button::primary`, `theme::container::floating`).

**Styles struct hierarchy:** `General` (backgrounds, borders), `Text` (primary/secondary/tertiary as `TextStyle`), `Buttons` (primary/secondary as `ButtonColors`), `Indicators` (accent/danger/warning/success). `TextStyle` has `color: Color` and optional `font_style: FontStyle`.

**TOML themes:** `themes/dark.toml` and `themes/light.toml`. `Theme::from_toml()` loads custom themes. `TextStyle` deserializes from either `"#hex"` or `{ color = "#hex", font_style = "bold" }`.

**Fonts:** Inter variable (regular + italic) for text, Lucide for icons. Constants in `src/font.rs`: `TEXT`, `TEXT_BOLD`, `TEXT_ITALIC`, `TEXT_SEMIBOLD`, `ICON`. Inter is set as `default_font`.

**Dark mode:** `src/appearance.rs` uses `mundy` to stream OS color scheme changes via `iced::advanced::graphics::futures::subscription::Recipe`.

## Migration context

This prototype is part of a migration from Tauri (React/TS frontend + Rust backend) to pure Rust with iced. The ~23k LOC Rust backend (providers, DB, body store, encryption) in `../src-tauri/core/` carries over as-is. The ~73k LOC TypeScript frontend gets replaced.

**Ecosystem:** Multi-window (Halloy, libcosmic, Kraken Desktop), rich text (Halloy), HTML email rendering (iced_webview_v2 + litehtml), platform support (all three ship on Windows + Linux).

**Forking iced:** Halloy and libcosmic both maintain iced forks. Halloy's fork adds X11 primary clipboard, shift-click text selection, and font styling helpers — all text selection/clipboard specifics, not architectural issues. We stay on upstream iced initially and fork only when we hit a concrete need.

**Email body rendering:** iced_webview_v2 with litehtml for table-based/basic emails, CEF fallback for complex HTML. Still needs testing against a real email corpus.

**Reference projects:** Git checkouts in `../research/`. Full analysis in `../docs/iced-migration-research.md`.
