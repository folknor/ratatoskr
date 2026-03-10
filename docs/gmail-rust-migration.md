# Gmail API → Rust Migration

**Completed**: March 2026

All Gmail API logic has been moved from TypeScript to Rust.

---

## What was built

### Rust Gmail stack (`src-tauri/src/gmail/`)

| File | Purpose |
|------|---------|
| `types.rs` | Gmail API serde structs (~25 types) |
| `client.rs` | `GmailClient` — `Arc<RwLock<TokenState>>`, reqwest, `&self` methods. `GmailState` holds per-account clients. |
| `api.rs` | ~20 Gmail REST methods (labels, threads, messages, drafts, history, send-as) |
| `parse.rs` | Gmail API response → `ParsedGmailMessage` (MIME tree walk, base64url decoding, attachment extraction) |
| `auth_parser.rs` | SPF/DKIM/DMARC parsing from Authentication-Results headers |
| `sync.rs` | Initial sync (labels → thread list → parallel fetch) and delta sync (History API, pending-ops filter) |
| `commands.rs` | 23 `#[tauri::command]` functions registered in `lib.rs` |

### Shared infrastructure (`src-tauri/src/provider/`)

| File | Purpose |
|------|---------|
| `crypto.rs` | AES-256-GCM encrypt/decrypt matching TS format. Used by all providers (Gmail, IMAP, future JMAP). |
| `token.rs` | `TokenState` struct + `refresh_oauth_token()` (generic, accepts any token endpoint) + `refresh_google_token()` (convenience wrapper for Google endpoint). Ready for reuse by other OAuth providers. |

| `http.rs` | `build_http_client()` (shared reqwest defaults), `RetryConfig`, `compute_retry_delay()` (Retry-After header + exponential backoff). Used by Gmail client. |

**No RFC 5322 message construction** — the TS composer builds raw RFC 5322 messages; Rust commands accept pre-built `raw_base64url` bytes. This is the correct boundary — there is no need for a `provider/message.rs`.

### TS layer (Rust-backed)

| File | How it uses Rust |
|------|-----------------|
| `gmailProvider.ts` | All methods call `invoke('gmail_*')` |
| `providerFactory.ts` | Creates `GmailApiProvider(accountId)` for non-IMAP accounts |
| `syncManager.ts` | Calls `gmail_sync_initial` / `gmail_sync_delta`, runs post-sync hooks |
| `tokenManager.ts` | Calls `gmail_init_client` on startup for each Gmail account |
| `emailActions.ts` | Routes Gmail operations through `invoke('gmail_*')` |
| `sendAs.ts` | Uses `invoke('gmail_fetch_send_as')` |
| `draftDeletion.ts` | Uses `invoke('gmail_list_drafts')` / `invoke('gmail_delete_draft')` |

---

## Key design decisions

1. **Rust owns tokens** — no dual control plane. `GmailState` holds canonical token state. TS only passes `account_id`.
2. **`&self` everywhere** — `GmailClient` is `Arc`-wrapped, `Clone` is cheap. Supports concurrent API calls (sync uses concurrency=10 for thread fetch).
3. **No shared `EmailProvider` trait** — Gmail commands are `gmail_*` prefixed. Extract a trait only when a second provider exists in Rust.
4. **Sync writes directly to DB** — no IPC per message. Rust writes to `ratatoskr.db` (threads, messages, labels, attachments), `bodies.db` (zstd-compressed), and tantivy search index.
5. **Pending-ops conflict check** — delta sync skips threads with entries in `pending_operations` table, preventing sync from overwriting optimistic local state.

---

## Sync vs queue write ordering

Two writers mutate local state: Rust sync (every 60s) and TS queue processor (every 30s). The `pending_operations` table is the coordination point — Rust sync checks it before overwriting any thread. SQLite's `Mutex<Connection>` serializes all writes.

---

## Remaining: `getGmailClient()` callers

`client.ts` and `getGmailClient()` are retained only for Calendar.

**Calendar** (different API): `googleCalendarProvider.ts` uses `GmailClient` for Google Calendar API calls (same OAuth token, different endpoint). Needs a separate Rust Calendar client.

**Migrated callers** (now use `invoke('gmail_*')` or `emailActions`):

| File | Before | After |
|------|--------|-------|
| `stores/labelStore.ts` | `client.createLabel/updateLabel/deleteLabel` | `invoke('gmail_create_label')` etc. |
| `components/search/CommandPalette.tsx` | `client.modifyThread` (spam) | `spamThread()` from emailActions |
| `components/layout/EmailList.tsx` | `client.listDrafts` | `invoke('gmail_list_drafts')` |
| `components/layout/MultiSelectBar.tsx` | `client.modifyThread/deleteThread` | `trashThread/archiveThread/spamThread/permanentDeleteThread` from emailActions |
| `services/snooze/scheduledSendManager.ts` | `client.sendMessage` | `invoke('gmail_send_email')` |
| `services/unsubscribe/unsubscribeManager.ts` | `client.sendMessage` | `invoke('gmail_send_email')` |

`getGmailClient()` can be deleted once Calendar gets its own Rust client.

---

## What stays in TS permanently (for now)

- OAuth flow (browser interaction)
- Sync timer (60s interval, multi-account orchestration)
- Post-sync hooks (filters, smart labels, notifications, AI categorization)
- `emailActions.ts` (optimistic UI, offline queue)
- `queueProcessor.ts` (dequeue + dispatch to Rust commands)
- `authParser.ts` (types + function still used by components)
- `messageParser.ts` (type-only: `ParsedMessage`, `ParsedAttachment` used by IMAP/filters/smart labels)
