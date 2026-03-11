# Ratatoskr

Tauri v2 desktop email client. Rust backend, React 19 frontend. ~23k lines Rust, ~73k lines TS.

## Commands

- `pnpm exec biome check --write` — lint+format (Biome, not ESLint/Prettier)
- `bunx tauri dev` — run dev (uses bun for tauri CLI, pnpm for deps)
- `vitest run` — tests; `vitest run path/to/file` for single file

## Gotchas that will break your code

**Multiple content stores**: Message metadata is not the whole story. Message bodies live outside the main `messages` table in `bodies.db` (zstd-compressed), and inline multipart images have their own attachment database as well. Use the Rust-side data access layer and existing fetch/write commands rather than assuming message content is fully stored in the main SQLite database.

**Four email providers in Rust**: `gmail_api`, `jmap`, `graph` (Microsoft), `imap`. All unified behind the `ProviderOps` trait (`provider/ops.rs`). Provider-agnostic commands in `provider/commands.rs`. The TS `EmailProvider` interface still exists but Graph throws — it uses Rust commands directly.

**Offline actions**: All email mutations go through `emailActions.ts` (optimistic UI + local DB + offline queue). Never call provider APIs directly from components.

**`src/core/`** is the facade layer — UI imports from `core/`, not from `services/db/` directly. `rustDb.ts` (38KB) wraps all Rust DB invoke calls.

## Lint rules that will surprise you

**Rust (edition 2024, strict clippy)**:
- `unwrap_used`: denied — use `?` or handle errors
- `too_many_arguments`: max 7
- `too_many_lines`: max 100 per function
- `cognitive_complexity`: denied
- `await_holding_lock`: denied
- `let_underscore_must_use`: denied (but fires on `tauri::command` macro expansion — pre-existing)

**TypeScript**:
- `verbatimModuleSyntax: true` — must use `import type` for type-only imports
- `erasableSyntaxOnly: true`
- `exactOptionalPropertyTypes: true` — `undefined` vs missing property matters
- `noUncheckedIndexedAccess: true`
- Target: ES2024, bundler module resolution
- Path alias: `@/*` → `src/*`

**Biome**:
- `useExplicitType`: all functions need explicit return types (off in tests)
- `noForEach`: use `for...of`
- `noExplicitAny`, `noNonNullAssertion`, `noShadow`: all errors
- `noFloatingPromises`, `noMisusedPromises`: enforced
- `noNamespaceImport`: no `import * as` (off in tests)
- `noBarrelFile`, `noReExportAll`, `noImportCycles`: all errors

## Tauri-specific

- **Capabilities**: New plugins need permissions in `src-tauri/capabilities/default.json`. Allowed windows: `main`, `splashscreen`, `thread-*`, `compose-*`
- **SQL plugin preload**: Must be array `["sqlite:ratatoskr.db"]`, not object
- **Emitter trait**: Must `use tauri::Emitter;` to call `.emit()` on windows
- **Single instance**: Must be first plugin registered
- **Minimize-to-tray**: Use `.on_window_event()` on Builder, not `window.on_window_event()`
- **Linux tray**: Uses `tray-item` crate (KSNI), not Tauri's built-in tray. `set_tray_tooltip` is a no-op on Linux
- **Window decorations**: macOS uses `titleBarStyle: "Overlay"` from config; Windows/Linux remove decorations programmatically in Rust

## Testing

Vitest + jsdom. `globals: true` (no imports for describe/it/expect). Tests colocated with source. Mocks in `src/test/mocks/`. Tests excluded from tsconfig compilation.

## Encryption

AES-256-GCM (`provider/crypto.rs`). Key file: `ratatoskr.key` (or legacy `velo.key`) in app data dir. Format: `base64(iv):base64(ct+tag)`. Falls back to zero-key if missing.

## Multi-window

Three window types via URL params in `main.tsx`: main app (no params), thread pop-out (`?thread=...&account=...`), composer pop-out (`?compose`). TanStack Router with hash history, lazy-loaded pages.
