# Ratatoskr UI Conventions

iced UI for the Ratatoskr email client (`crates/app/`). Uses iced 0.15-dev (Halloy's fork) against a seeded test database.

**iced fork:** `https://github.com/folknor/iced`, branch `sluggrs`. Local checkout at `/home/folk/Programs/sluggrs/repos/iced`. When you need to check what API is available in our iced version, read files in that directory — do not assume upstream iced APIs exist.

## App module structure (`crates/app/src/`)

`main.rs` is a thin dispatch layer (~1300 lines). All feature logic lives in handler modules.

### Handler modules (`handlers/`)

Each feature area has its own handler file with `impl App` blocks:

| File | Owns | Key methods |
|---|---|---|
| `handlers/calendar.rs` | Calendar event CRUD, overlay state | `handle_calendar()`, `reload_calendar_events()` |
| `handlers/keyboard.rs` | Key events, chord resolution | `handle_key_event()`, `try_resolve_single_chord()` |
| `handlers/commands.rs` | Command execution dispatch | `handle_execute_command()`, `build_command_args()` |
| `handlers/palette.rs` | Command palette UI state machine | `handle_palette()`, `palette_confirm()` |
| `handlers/search.rs` | Search + pinned searches | `handle_search_execute()`, `handle_select_pinned_search()` |
| `handlers/pop_out.rs` | Pop-out windows, compose | `handle_pop_out_message()`, `open_compose_window()` |
| `handlers/signatures.rs` | Signature save/delete/load | `handle_save_signature()`, `load_signatures_into_settings()` |
| `handlers/contacts.rs` | Contact/group CRUD dispatch | `handle_save_contact()`, `handle_load_groups()` |
| `handlers/accounts.rs` | Account wizard events | `handle_add_account_event()` |

**Adding new handler methods:** Create methods on `App` in the appropriate handler file. Private fields of `App` are accessible from handler modules (they're descendant modules of the crate root). Import types with `use crate::` paths.

**Adding new `Message` variants:** Add to the `Message` enum in `main.rs`, add the dispatch arm in `update()`, and implement the handler method in the appropriate handler file.

### DB modules (`db/`)

| File | Owns |
|---|---|
| `db/connection.rs` | `Db` struct, `open()`, `with_conn`/`with_write_conn` helpers |
| `db/accounts.rs` | `get_accounts()`, `get_labels()` |
| `db/threads.rs` | Thread/message/attachment/body queries |
| `db/calendar.rs` | Calendar event CRUD |
| `db/contacts.rs` | Contact/group queries for settings |
| `db/palette.rs` | Label/folder queries for command palette |
| `db/pinned_searches.rs` | Pinned search CRUD |
| `db/types.rs` | All DB types (`Thread`, `Account`, `ThreadMessage`, etc.) |

### UI modules (`ui/`)

Each component has its own file. Components implement the `Component` trait from `component.rs` with `type Message` and `type Event` associated types. Internal messages stay in `update()`, outward signals emit as events to the parent `App`.

Components: `sidebar.rs`, `thread_list.rs`, `reading_pane.rs`, `status_bar.rs`, `settings/` (module), `add_account.rs`.

Non-component UI: `calendar.rs`, `calendar_month.rs`, `calendar_time_grid.rs`, `palette.rs`, `right_sidebar.rs`, `theme.rs`, `layout.rs`, `widgets.rs`, `popover.rs`, `emoji_picker.rs`, `token_input.rs`.

## Gotchas

**`Padding::from` with mixed types:** `Padding::from([0, CONSTANT])` won't compile if `CONSTANT` is `f32` — Rust infers the array as `[i32; 2]`. Always use `[0.0, CONSTANT]` to keep both elements `f32`.

**`iced::Font::DEFAULT` is not Inter:** We set `default_font(font::TEXT)` which is `Font::new("Inter")`. If you construct a font with `iced::Font { weight, ..iced::Font::DEFAULT }` it will NOT use Inter. Always spread from `font::TEXT` instead: `iced::Font { weight, ..font::TEXT }`.

**`iced::mouse` doesn't re-export the `click` submodule.** `iced::mouse` (from `iced::core::mouse` via `iced/src/lib.rs`) only re-exports `Button`, `Cursor`, `Event`, `Interaction`, `ScrollDelta`. To access `Click`, `click::Kind` (Single/Double/Triple), or anything else in the `click` module, use `iced::advanced::mouse::click::Click` and `iced::advanced::mouse::click::Kind`. The `iced::advanced` module re-exports the full `iced::core::mouse` which includes `pub mod click`.

**`scrollable::scroll_to()` does not exist in this iced fork.** There is no public function to programmatically scroll a `Scrollable` to a specific offset. The internal `State` has `scroll_by()` but it's not exposed as a top-level function. If you need scroll-to-item behavior, you'll need a different approach (e.g., widget operations or state manipulation).

**Button `text_color` doesn't reach children with explicit `.style()`.** If you set `text_color` on a button style but the `text()` or icon inside has its own `.style(some_fn)`, the explicit style wins. The button's `text_color` only affects children that don't override it. This means changing a button style's color has no visible effect when all children set their own style — you have to change the text/icon styles too.

**Popover menu width is constrained to the trigger's width.** The `PopoverOverlay` layout uses `base_bounds.width` as the menu's max width. If the trigger is `Length::Shrink`, the menu will be tiny. For narrow triggers (like the `select` widget), set an explicit width on the trigger (e.g., `SELECT_MIN_WIDTH`) so the menu has room.

**`height()` on containers is fixed, not minimum.** There's no `min_height()` on containers in this iced version. If you need rows to be "at least X tall but grow for bigger content," use a shared `height()` constant and accept that all rows are that exact height. Use different constants for different row types (e.g., `SETTINGS_ROW_HEIGHT` vs `SETTINGS_TOGGLE_ROW_HEIGHT`).

**Scrollable clips shadows.** If a container inside a `scrollable` has a `shadow`, the shadow will be clipped at the scrollable's bounds. Don't put padding on the outer container around the scrollable — put it on an inner container *inside* the scrollable so the padding becomes part of the scrolled content and gives the shadow room to render.

**Palette background scale (dark mode).** `base` is the darkest. Each step lightens by a fixed deviation: `base` (0%) → `weakest` (3%) → `weaker` (7%) → `weak` (10%) → `neutral` (12.5%) → `strong` (15%) → `stronger` (17.5%) → `strongest` (20%). In light mode the direction reverses. Use this to create visual depth hierarchy — e.g., fieldsets at `base`, content area at `weakest`, sidebar at `weaker`.

**Hover backgrounds are always one palette step away from the element's resting color.** If a row rests on `base`, its hover is `weakest`. If it rests on `weakest`, its hover is `weaker`. Never skip steps — jumping two or more steps makes hover effects feel heavy. This applies to nav buttons, setting rows, dropdown items, chip buttons, and any other interactive surface.

**`mouse_area.on_move` gives coordinates relative to that mouse_area's bounds.** If you wrap individual list items in mouse_areas and drag past one item's bounds, `on_move` stops firing or reports clamped coordinates. For drag-to-reorder, wrap the *entire list* in a single mouse_area so cursor Y maps directly to item index via `(point.y / row_height)`.

**`iced::widget::stack` doesn't block events on lower layers.** Stacking an overlay on top of content doesn't prevent clicks/hovers from reaching the content underneath. Insert a `mouse_area` between the layers that captures all events (set `on_press` to consume clicks). This acts as an event blocker.

**`responsive` closure is `Fn`, not `FnOnce`.** You can't move `Element`s into a `responsive(|size| ...)` closure because it may be called multiple times. Use fixed offsets or store computed sizes in state instead.

**Buttons without `on_press` don't show hover states.** Iced treats them as disabled — no hover background, no cursor change. If you need a hover effect, the button must have `on_press` even if the action is just focusing a child widget.

**Buttons hardcode the pointer cursor.** You cannot override the cursor to e.g. `Interaction::Text` for an inline edit row. Wrapping the button in a `mouse_area` with a different `interaction` won't help — the button's `mouse_interaction` wins for its bounds.

**`text_input` without `on_input` is fully disabled.** No focus, no selection, no cursor — completely inert. To make a read-only but selectable/copyable input, provide a no-op `on_input` (e.g. `|_| Message::Noop`).

**Programmatic focus uses `widget::operation::focus(id)`.** Not `text_input::focus`. The ID is `widget::Id` (accepts `String` via `From<String>`). Set the ID on the text input with `.id("my-id")` and return `iced::widget::operation::focus("my-id".to_string())` as a `Task` from `update()`.

**Overlay `mouse_interaction` must never return `None` over its bounds.** iced uses the overlay's `mouse_interaction()` return value to decide whether to pass the cursor to base widgets. If it returns `Interaction::None` (e.g. cursor is in spacing gaps between menu items), iced passes the real cursor to base widgets, causing their hover states to bleed through. Return `Interaction::Idle` instead when the cursor is over the overlay bounds but between child widgets.

**Popover overlay positioning must include the `translation` vector.** In `Widget::overlay()`, `layout.position()` returns coordinates relative to the parent widget, not the window. Add the `translation` parameter: `layout.position() + translation`. Without this, popups misposition at non-1.0 scale factors and inside scrollables.

## PaneGrid internals and scale factor

### How iced computes window/layout sizes

iced has three size concepts in the rendering pipeline:

1. **Physical size** — actual pixels on screen (from winit's `WindowEvent::Resized`)
2. **Viewport scale factor** — `system_dpi * app_scale_factor` (our app auto-detects scale from monitor DPI via `display-info` crate; see `src/display.rs`)
3. **Logical size** — `physical_size / viewport_scale_factor` — this is what the layout engine uses

The `window::resize_events()` subscription reports the **logical size** (already divided by the full scale factor, including the app's). This is the same size the PaneGrid uses for layout. So `resize_event.width == PaneGrid_layout_width`. Do NOT divide by the app scale factor again — it's already factored in.

The chain: `winit::Resized(physical)` → `window::State` creates `Viewport::with_physical_size(physical, system_dpi * app_scale)` → `conversion::window_event` calls `physical.to_logical(viewport.scale_factor())` → that logical size is what `resize_events()` emits AND what `ui.relayout(logical_size, ...)` uses for layout.

### PaneGrid resize events and ratio semantics

When dragging a split divider, iced computes the new ratio as:

```
ratio = (cursor_position - region_origin) / region_width
```

Where `region` comes from `Node::split_regions()`. For a nested split like ours (Sidebar | ThreadList | ReadingPane):

- **Outer split ratio**: relative to the full PaneGrid width (region = full bounds)
- **Inner split ratio**: relative to the right portion's width (region = everything after the sidebar)

The `ResizeEvent` sends this ratio directly — no pre-clamping. iced's `min_size` only affects the visual layout (via `axis.split()` which clamps widths), not the event ratio.

### How `axis.split()` works internally

```rust
let width_left = (rect.width * ratio - spacing / 2.0)
    .round()
    .max(min_size_a)
    .min(rect.width - min_size_b - spacing);
```

For nested splits, `min_size_a` and `min_size_b` account for pane count: `min_size * (pane_count) + spacing * (split_count)`. So with `.min_size(200)`, the right portion (containing 2 panes and 1 split) gets `min_size_b = 200 * 2 + spacing * 1 = 401`.

### Per-pane minimum enforcement

iced's PaneGrid only supports a single global `min_size` for all panes. Per-pane minimums require clamping the ratio in `update()` before calling `State::resize()`. Important: this clamping must also run on `WindowResized`, not just `PaneResized` — otherwise shrinking the window can push panes below their minimum, and the constraint only snaps back on the next drag.

### Variable shadowing trap

When destructuring `Node::Split { id, ratio, .. }` inside a function that also takes a `ratio: f32` parameter, the struct field shadows the parameter. Name the struct field `ratio: current_ratio` to avoid silently clamping the stored ratio instead of the new drag ratio.

## Layout module (`src/ui/layout.rs`)

All sizing, spacing, padding, and radii are centralized here. Views import `use crate::ui::layout::*` and reference named constants. **No magic numbers in view or widget code** — every `.size()`, avatar diameter, border radius, and padding must reference a layout constant.

**Spacing scale** (geometric): `SPACE_XXXS` (2) → `SPACE_XXS` (4) → `SPACE_XS` (8) → `SPACE_SM` (12) → `SPACE_MD` (16) → `SPACE_LG` (24) → `SPACE_XL` (32) → `SPACE_XXL` (48) → `SPACE_XXXL` (64).

**Type scale:** `TEXT_XS` (10) → `TEXT_SM` (11) → `TEXT_MD` (12) → `TEXT_LG` (13) → `TEXT_XL` (14) → `TEXT_TITLE` (16) → `TEXT_HEADING` (18). Every `text(...).size()` must use one of these.

**Icon sizes:** `ICON_XS` (10) → `ICON_SM` (11) → `ICON_MD` (12) → `ICON_LG` (13) → `ICON_XL` (14). Every `icon::foo().size()` must use one of these.

**Avatar sizes:** `AVATAR_DROPDOWN_ITEM` (20), `AVATAR_DROPDOWN_TRIGGER` (24), `AVATAR_THREAD_CARD` (28), `AVATAR_MESSAGE_CARD` (32), `AVATAR_CONTACT_HERO` (56). Every `avatar_circle()` call must use one of these.

**Leading slot widths:** `SLOT_DROPDOWN`. When a list item has an icon or avatar on the left, wrap it in a fixed-size container so all labels align.

**Border radii:** `RADIUS_SM` (4), `RADIUS_MD` (6), `RADIUS_LG` (8). Every `border::rounded()` or `radius:` value must use one of these.

**Padding presets** are named by role: `PAD_ICON_BTN`, `PAD_NAV_ITEM`, `PAD_BUTTON`, `PAD_SIDEBAR`, `PAD_PANEL_HEADER`, `PAD_TOOLBAR`, `PAD_CONTENT`, `PAD_CARD`, `PAD_THREAD_CARD`, `PAD_INPUT`, `PAD_SECTION_HEADER`, `PAD_COLLAPSIBLE_HEADER`, `PAD_STAT_ROW`, `PAD_BADGE`, `PAD_DROPDOWN`, `PAD_BODY`.

**Panel widths:** `SIDEBAR_WIDTH` (180), `THREAD_LIST_WIDTH` (400), `RIGHT_SIDEBAR_WIDTH` (240). Thread card height: `THREAD_CARD_HEIGHT` (68). Label dots: `LABEL_DOT_SIZE` (6). Right sidebar auto-collapse: `RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH` (1200).

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

## Theme system (`src/ui/theme.rs`)

Uses iced's built-in `Theme::custom(name, Seed)` with 6 seed colors. `Seed` (background, text, primary, success, warning, danger) auto-generates a full `Palette` via OKLCh derivation: 8 background levels, primary/secondary/success/warning/danger with base/weak/strong variants. Access via `theme.palette()`.

**Built-in themes:** 21 palettes in `THEMES` array (Light, Dark, Dracula, Nord, Solarized, Gruvbox, Catppuccin, Tokyo Night, Kanagawa, etc.). Theme files are 8-line TOMLs with 6 hex colors.

**Custom styles:** ~30 style functions for email-specific widgets (thread cards, nav buttons, badges, popovers, etc.). All use `theme.palette()` to derive colors from the current theme.

**Fonts:** Inter variable (regular + italic) for text, Lucide for icons. Constants in `src/font.rs`: `TEXT`, `TEXT_BOLD`, `TEXT_ITALIC`, `TEXT_SEMIBOLD`, `ICON`. Inter is set as `default_font`.

**Dark mode:** `src/appearance.rs` uses `mundy` to stream OS color scheme changes via `iced::advanced::graphics::futures::subscription::Recipe`.
