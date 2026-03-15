# iced-proto

Prototype iced UI for the Ratatoskr email client. Renders a four-pane email interface using iced 0.15-dev (Halloy's fork) against a seeded test database.

## Commands

- `python3 seed-db.py [thunderbird.sqlite] [output-dir]` — seed a test DB from Thunderbird's `global-messages-db.sqlite` (defaults: `./thunderbird.sqlite` → `~/.local/share/com.velo.app/ratatoskr.db`)
- `cargo run` — run the prototype (requires a seeded `ratatoskr.db` in `~/.local/share/com.velo.app/`)
- `cargo check` — type-check
- `cargo clippy` — lint

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

## Layout module (`src/ui/layout.rs`)

All spacing, padding, and panel widths are centralized here. Views import `use crate::ui::layout::*` and reference named constants — no magic numbers in view code.

**Spacing scale** (4px base grid): `SPACE_XXS` (2) → `SPACE_XS` (4) → `SPACE_SM` (6) → `SPACE_MD` (8) → `SPACE_LG` (10) → `SPACE_XL` (12) → `SPACE_XXL` (16) → `SPACE_XXXL` (20).

**Padding presets** are named by role, not by values: `PAD_ICON_BTN`, `PAD_NAV_ITEM`, `PAD_BUTTON`, `PAD_SIDEBAR`, `PAD_PANEL_HEADER`, `PAD_TOOLBAR`, `PAD_CONTENT`, `PAD_CARD`, `PAD_THREAD_CARD`, `PAD_INPUT`, `PAD_SECTION_HEADER`, `PAD_STAT_ROW`, `PAD_BADGE`, `PAD_ACCOUNT`.

**Panel widths:** `SIDEBAR_WIDTH` (180), `THREAD_LIST_WIDTH` (280), `CONTACT_SIDEBAR_WIDTH` (240).

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
