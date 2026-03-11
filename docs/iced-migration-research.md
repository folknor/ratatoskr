# Iced Migration Research

Research into migrating Ratatoskr's UI from Tauri (React/TypeScript frontend + Rust backend) to iced (pure Rust GUI). Conducted March 2026.

## Motivation

1. **WebKitGTK on Linux** — unreliable, poorly maintained, inconsistent rendering across distros. Tauri devs have acknowledged the problem and are actively exploring alternatives (Servo, CEF, QtWebKit).
2. **Compile times** — Tauri pulls in the entire GTK stack. Incremental builds during development are too slow to iterate effectively.
3. **Dual-language friction** — maintaining ~73k lines of TypeScript alongside ~23k lines of Rust means constant context-switching and bridge overhead.

### Non-goals

- macOS support
- Non-Latin text rendering (acceptable to degrade)

### Priorities

- Performance
- Windows support
- Linux support

## Compile Time Comparison

Measured on AMD Ryzen 5 5600G (12 threads), rustc 1.96.0-nightly, Linux. Same machine, same session.

| Metric | Tauri (src-tauri) | iced (prototype) | Factor |
|--------|-------------------|------------------|--------|
| **Clean build** | 4m 59s | 1m 47s | **2.8x faster** |
| **Incremental (no change)** | 1m 38s | 0.19s | **516x faster** |
| **Incremental (touch source)** | 1m 37s | 0.92s | **106x faster** |

### Tauri: top crates by compile time

| Time | Crate | Category |
|------|-------|----------|
| 114.1s | ratatoskr (own code) | Own code |
| 84.4s | tauri-utils | Tauri |
| 49.3s | gtk | GTK |
| 46.0s | tauri-utils | Tauri |
| 42.4s | zbus | D-Bus/GTK |
| 39.4s | syn | Proc macros |
| 36.9s | bon-macros | Proc macros |
| 36.0s | tauri-codegen | Tauri |
| 28.8s | gio | GTK |
| 25.9s | tauri-build | Tauri |

GTK stack total: ~112s. Tauri infra total: ~193s. These are completely eliminated by iced.

### iced: top crates by compile time

| Time | Crate | Category |
|------|-------|----------|
| 40.4s | x11rb-protocol | X11 |
| 40.3s | naga | wgpu shaders |
| 28.9s | read-fonts | Font rendering |
| 27.5s | syn | Proc macros |
| 27.4s | wgpu-core | GPU |
| 23.1s | wayland-protocols | Wayland |
| 22.2s | skrifa | Font rendering |
| 21.3s | winit | Windowing |
| 21.0s | swash | Font rendering |
| 20.5s | wgpu-hal | GPU |

### Why incremental is so different

Tauri's incremental builds recompile the entire `ratatoskr` crate (114s) on every change because `tauri-codegen` and the macro-heavy Tauri command system invalidate caching aggressively. iced has no equivalent code generation layer — your source compiles directly, and only changed files recompile.

## Research Projects

Five open-source projects and one closed-source production app were explored, plus a blog post on web rendering in iced.

### Kraken Desktop (crypto trading terminal)

- **Source:** Closed-source. https://kraken.com/desktop
- **Iced version:** Unknown (Kraken sponsors iced development)
- **Platform support:** Windows, macOS, Linux
- **Launched:** Late 2024 after private beta

**What it is:** A professional cryptocurrency trading terminal serving 800+ markets. Real-time order books, live price charts, depth charts, ladder trading, iceberg orders, trailing stops. Built from scratch in Rust + iced with no webviews.

**What it proves:**
- **Real-time streaming data** at trading-grade latency — order books and charts updating continuously without frame drops
- **Multi-window** — traders run multiple windows simultaneously across monitors
- **Complex dynamic layouts** — up to 48 customizable modules per board, with shareable layout templates
- **Custom drawing/charting** — built their own technical analysis drawing library natively in iced
- **Low resource usage** — explicitly marketed as low CPU/GPU/memory vs web-based competitors (Electron, iframe-based terminals)
- **Cross-platform shipping product** — real traders using this daily on all three desktop platforms
- **Audio notifications, custom themes, price alerts** — full desktop app feature set

**Takeaways:** This is the strongest existence proof for iced at production scale. A trading terminal with live streaming data across 800+ markets and 48-module customizable layouts is arguably harder than an email client in terms of real-time rendering demands. Kraken's sponsorship of iced also means the framework has funded, professional investment in its continued development — not just community volunteers.

### Sniffnet (network traffic monitor)

- **LOC:** ~27,500 Rust
- **Iced version:** 0.14.0 (upstream, not forked)
- **Features used:** tokio, svg, advanced, lazy, image
- **UI complexity:** 8 pages, 3 settings sub-pages, 5 custom components
- **Multi-window:** No (single window with overlays)
- **Platform support:** Windows, macOS, Linux
- **Custom widgets:** 1 (`EllipsizedText` for text truncation using iced's `advanced` feature)

**Takeaways:** Proves iced works for moderately complex desktop apps. Lists are built manually with Column/Row (no built-in table widget). Theming is comprehensive (12 themes + custom JSON). Uses `plotters-iced2` for charts. Strict clippy config matches Ratatoskr's. Falls back to `tiny-skia` CPU renderer on old hardware via `ICED_BACKEND` env var.

### Halloy (IRC client)

- **LOC:** ~32,863 Rust (main crate), ~6,283 in custom widgets alone
- **Iced version:** 0.15.0-dev (forked: `squidowl/iced`)
- **Features used:** wgpu, tiny-skia, tokio, lazy, advanced, image, svg, wayland, x11, web-colors
- **UI complexity:** Dashboard with pane grid, 7 buffer types, command bar, modals, toasts
- **Multi-window:** Yes (pop-out windows for individual buffers)
- **Platform support:** Windows, macOS, Linux (all with native integration)
- **Custom widgets:** 14+ (selectable_rich_text, combo_box, context_menu, decorate, double_click, anchored_overlay, modal, color_picker, reaction_row, etc.)

**Takeaways:** Most relevant comparison — a real-time messaging app with rich text, message lists, text composition, and multi-account support. Key patterns:

- **Virtualized scrolling** with buffer pages, debounced scroll events, and batch height measurement
- **Rich text** via `selectable_rich_text` widget (968 lines) wrapping iced's `text::Span` with selection, links, and context menus
- **Text editor** uses iced's `text_editor` as base, layers Emacs keybindings, command history, and multi-category autocompletion (commands, usernames, emojis with fuzzy matching via `nucleo-matcher`)
- **Pane grid** for multi-pane layout with drag-and-drop rearrangement
- **Forked iced** — needed patches upstream hasn't merged
- **Platform-specific code:** macOS transparent titlebar, Linux KSNI tray + XDG portals, Windows icon embedding
- **Theme system** with dynamic light/dark tracking via `mundy` crate

### libcosmic (COSMIC desktop toolkit by System76)

- **LOC:** ~43,000 Rust
- **Iced version:** Forked (pop-os/iced, shipped as git submodule)
- **Widgets:** 50+ (tables, nav bars, segmented buttons, dropdowns, color picker, calendar, toaster, context menus, drag-and-drop, etc.)
- **Multi-window:** Yes (feature-gated, full support including Wayland subsurfaces)
- **Platform support:** Linux-first (D-Bus, XDG portals, Wayland-native). Windows/macOS are afterthoughts.

**Takeaways:** Most ambitious iced project in existence. Proves iced can support a full desktop environment's worth of applications. However:

- Tightly coupled to COSMIC/System76 ecosystem
- Ships a forked iced for tight integration
- Windows support is explicitly not a priority
- No HTML/web rendering — not their use case
- Accessibility is experimental
- Many TODO comments in widget styling

### frostmark (HTML/Markdown renderer for iced)

- **LOC:** ~1,660 Rust
- **Iced version:** 0.14 with `advanced` feature
- **Purpose:** Converts HTML/Markdown into native iced widget trees (Column, Row, RichText, buttons, etc.)

**Takeaways:** Handles structured content well — headings, lists, tables, code blocks, links, interactive details/summary elements. Uses `html5ever` for parsing. **Not suitable for arbitrary email HTML** — complex CSS layouts, marketing emails with inline styles, and image-heavy content would break it. Best described as a documentation renderer.

### iced_webview_v2 (webview widget for iced)

- **Version:** 0.1.4 (Feb 2026)
- **Iced version:** 0.14
- **Purpose:** Embed HTML/CSS/JS content inside iced applications

**Four rendering backends:**

| Engine | CSS Support | JavaScript | Binary Size | Build Deps | Status |
|--------|------------|------------|-------------|------------|--------|
| litehtml | Basic flexbox, tables, no grid | No | Small | clang/libclang | Stable |
| Blitz | Full modern CSS (flexbox, grid) | No | Moderate | Git-only (Stylo) | Pre-alpha |
| Servo | Full HTML5/CSS3 | Yes (SpiderMonkey, crashes on heavy JS) | 50-150 MB | fontconfig, cmake, clang, nasm | Experimental |
| CEF | Full Chromium | Yes (V8) | 200-300 MB | Downloads Chromium at build time | Production-ready |

**Two rendering architectures:**
- **Image buffer path** (litehtml, Blitz): Engine rasterizes to RGBA pixel buffer, displayed via iced `image::Handle`
- **Shader widget path** (Servo, CEF): Engine renders to GPU texture via `queue.write_texture()`, avoids texture cache churn during scrolling

**For email rendering:** litehtml is the pragmatic default — lightweight, stable, handles table-based layouts that most marketing emails use. CEF is the fallback for full web compatibility. Includes an `email.rs` example demonstrating HTML email rendering.

**Limitations:** No raw JS API for static engines, no DOM access, keyboard input not fully wired for Blitz, text selection not queryable from Servo.

### Blog Post: "Web Rendering in Iced: What Actually Works"

Source: gofranz.com (author of iced_webview_v2)

Confirms the four-engine assessment above. Key additional details:
- litehtml is best for "HTML emails, documentation pages, basic content display"
- Blitz skips `:hover` CSS re-renders for performance
- Servo's SpiderMonkey "crashes on pages with heavy JavaScript"
- CEF is the "only option right now if you need full web compatibility without proprietary licensing"
- No incremental rendering in pure Rust options (full viewport re-rasterized on change)

## Key Findings

### What iced can do today

- **Complex desktop apps** — Kraken Desktop (trading terminal), Halloy (IRC client), and libcosmic (desktop toolkit) prove this at scale
- **Real-time performance** — Kraken Desktop handles live-streaming order books and charts across 800+ markets with 48-module layouts
- **Multi-window** — works, both via pop-outs (Halloy) and full multi-window (libcosmic, Kraken)
- **Rich text display** — spans with inline formatting, colors, fonts, links, selection
- **Virtualized scrolling** — requires custom implementation but Halloy shows the pattern
- **Text editing** — iced's `text_editor` works as a base, needs customization for advanced features
- **Platform-specific integration** — system tray, native dialogs, theme detection all solved
- **HTML email rendering** — viable via iced_webview_v2 + litehtml (or CEF for complex emails)
- **Compile times** — 100x faster incremental builds vs Tauri (measured)

### What requires significant effort

- **Custom widgets** — expect to write 10-15 custom widgets for a complex app. Both Halloy and libcosmic invested heavily here.
- **Forking iced** — both production-grade complex apps (Halloy, libcosmic) maintain their own iced fork. This is likely necessary at our complexity level.
- **Scroll performance** — virtualization, height caching, debouncing, and batch measurement are all manual work
- **Text input polish** — iced's built-in text input is functional but needs layers of custom behavior for production quality (autocompletion, keybindings, history)
- **No built-in table widget** — must build or adapt from libcosmic

### Risks

- **Forked iced maintenance** — staying on a fork means rebasing against upstream, resolving conflicts, and potentially diverging over time
- **HTML email fidelity** — litehtml handles basic/table-based emails but may struggle with modern CSS-heavy emails. Need to test against real-world email corpus.
- **Windows polish** — iced's Windows support works. Kraken Desktop and Halloy both ship on Windows, which significantly reduces this risk.
- **Ecosystem maturity** — iced is pre-1.0. API changes between versions are common (Halloy is on 0.15-dev, sniffnet on 0.14).
- **Accessibility** — limited. libcosmic has experimental `a11y` support but it's incomplete.

## Proposed Architecture

If proceeding, the architecture would be:

```
Ratatoskr (iced)
├── App shell (iced)
│   ├── Sidebar (account list, folder tree)
│   ├── Message list (custom widget, virtualized)
│   ├── Thread view (custom widget)
│   ├── Compose editor (iced text_editor + custom layers)
│   └── Settings / modals
├── Email body pane
│   └── iced_webview_v2
│       ├── litehtml (default, lightweight)
│       └── CEF (fallback for complex HTML)
├── Rust backend (existing, largely unchanged)
│   ├── Provider trait (Gmail, JMAP, Graph, IMAP)
│   ├── SQLite database layer
│   ├── Body store (zstd-compressed)
│   └── Encryption (AES-256-GCM)
└── Multi-window
    ├── Main window
    ├── Thread pop-outs
    └── Compose pop-outs
```

The Rust backend (~23k LOC) would transfer almost entirely. The TypeScript frontend (~73k LOC) would be replaced with Rust iced code. Based on Halloy's ratios (33k Rust for a comparable messaging UI), the iced frontend would likely be 30-45k lines of Rust.

## Prototype

A working iced prototype exists at `iced-proto/` that reads a real Ratatoskr database and displays accounts, labels, and a scrollable thread list. It was tested with 17.7k threads (23.5k messages) seeded from a real Thunderbird profile.

**Files:**
- `iced-proto/src/main.rs` — iced app with sidebar (accounts, labels) and thread list
- `iced-proto/src/db.rs` — standalone DB layer (no Tauri dependency)
- `iced-proto/seed-db.py` — seeds a Ratatoskr DB from Thunderbird's `global-messages-db.sqlite`

**What it demonstrates:**
- Direct reuse of the existing SQLite database via rusqlite
- Async queries via tokio `spawn_blocking` (same pattern as `src-tauri/src/db/mod.rs`)
- iced's Elm-architecture update/view loop
- Sidebar with account switching and label/folder selection
- Scrollable thread list with sender, subject, snippet, date, read/unread styling, and indicators

**Observed performance:**
- 1000 thread entries render instantly in a scrollable column — no virtualization needed at this scale
- App startup is near-instant (DB open + query + render)
- No perceptible lag when switching accounts or labels

### Iced fork question

Halloy (IRC client) and libcosmic (COSMIC desktop) both maintain iced forks. Halloy's fork adds X11 primary clipboard (copy-on-select), shift-click text selection expansion, and font styling helpers — all text selection/clipboard features specific to their IRC use case, not architectural issues with iced. We can likely stay on upstream iced initially and fork only when we hit a concrete need.

## Decision

**Proceeding with iced migration.** The research and prototype confirm:

1. **Compile times are transformative** — 0.92s vs 97s incremental builds. This alone justifies the migration for developer productivity.
2. **Rendering performance is excellent** — 1000 thread entries render instantly without virtualization. Kraken Desktop proves iced handles far more demanding real-time workloads.
3. **The ecosystem covers our needs** — multi-window (Halloy, libcosmic, Kraken), rich text (Halloy), HTML email rendering (iced_webview_v2 + litehtml), platform support (all three ship on Windows + Linux).
4. **The backend transfers cleanly** — the prototype proved the DB layer works standalone with zero Tauri dependency. The ~23k LOC Rust backend (providers, DB, body store, encryption) carries over as-is.
5. **No WebKitGTK** — the primary pain point is eliminated entirely.

## Next Steps

1. ~~Measure current compile times~~ Done. 97s incremental (Tauri) vs 0.92s (iced).
2. ~~Build minimal iced prototype~~ Done. Working with real data.
3. ~~Validate rendering performance~~ Done. 1000 items, instant.
4. **Test email rendering** — run iced_webview_v2's email example against a corpus of real emails
5. **Evaluate compile times at scale** — the prototype is tiny; monitor as codebase grows
6. **Test on Windows** — verify iced + iced_webview_v2 + litehtml works acceptably
7. **Plan the migration** — define module boundaries, identify which TS UI code maps to which iced widgets, prioritize screens
