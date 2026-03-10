# JMAP Rust Provider — Completed

**Completed**: March 2026

Rust-native JMAP (RFC 8620/8621) email provider using `jmap-client` 0.4 crate. Basic auth only. 25 Tauri commands (`jmap_*` prefix). Full sync engine (initial + delta via state strings). All email actions, send/drafts, attachment download. TS integration wired through providerFactory, syncManager, emailActions. Account setup UI (`AddJmapAccount.tsx`) with auto-discovery.

### What was built

- `src-tauri/src/jmap/` — 7 modules: `client.rs`, `sync.rs`, `commands.rs`, `parse.rs`, `mailbox_mapper.rs`, `auto_discovery.rs`, `ops.rs`
- `src/services/email/jmapProvider.ts` — `JmapProvider` implementing `EmailProvider`
- `src/components/accounts/AddJmapAccount.tsx` — 3-step account setup wizard
- DB migration v25: `accounts.jmap_url` column + `jmap_sync_state` table

### Key design decisions (still relevant)

- **Thread-level actions**: JMAP has no thread mutations. Rust enumerates emails via `Email/query(inThread)` then batch-mutates with `Email/set`. Queue stores `threadId` as `resource_id`.
- **Pending-ops conflict check**: Delta sync skips threads with entries in `pending_operations`.
- **jmap-client Issue #18**: `Email/set` uses `false` instead of `null` to remove `mailboxIds`/`keywords` patch entries. May need vendoring if this breaks against strict servers.
- **No `Authentication-Results`**: JMAP doesn't expose transport-level auth headers. `auth_results` is NULL for JMAP messages.
- **Blob ID reuses `gmail_attachment_id` column**: JMAP blob IDs are stored in the existing `gmail_attachment_id` column on the `attachments` table rather than adding a new column.
- **`email_changes` uses `None` for max_changes** (server-decided batch size), not a fixed number. `mailbox_changes` uses `500`.

### Implementation divergences from original plan

- **Send is not batched**: The plan described a single batch request (Email/import + EmailSubmission/set + on-success keyword update). Implementation uses sequential calls: `email_import()` → `email_submission_create()` → `email_set_keyword()`. The batch pattern requires careful back-reference wiring that `jmap-client`'s high-level API doesn't expose cleanly.
- **Email actions are per-email sequential, not batched `Email/set`**: The plan showed building a single `Email/set` request mutating all emails in a thread at once. Implementation uses `jmap-client`'s convenience methods (`email_set_mailbox`, `email_set_keyword`) which make one API call per email. Works correctly but could be optimized into batch requests later.
- **`JmapClient` wraps `Arc<Client>` and is `Clone`**: The plan showed a plain struct with `&` references. Implementation uses `Arc` so the client can be cloned out of the `RwLock` (avoids holding the read lock across await points).
- **`JmapState` takes `encryption_key`**: The plan showed `JmapState::new()` with no args. Implementation passes the encryption key at construction (same key loaded once in `lib.rs` setup, shared with `GmailState`).
- **Initial sync pagination**: The plan described paginated `Email/query`. Implementation re-queries each loop iteration with position offset (no JMAP `position` parameter used), which works but re-fetches the full ID list each time. Could be optimized.
- **25 commands, not 23**: Final command count is 25 (the plan estimated ~23). Extra commands from `jmap_get_profile`, `jmap_discover_url` as separate commands, and folder CRUD.

### jmap-client 0.4 API gotchas (learned during implementation)

These are non-obvious behaviors of the `jmap-client` crate that will matter if the code is modified:

- **Getting all mailboxes**: `mailbox_get(id, props)` fetches ONE mailbox. To get all, use the builder pattern: `request.get_mailbox()` with no ID set.
- **`mb.role()`** returns `Role` directly (not `Option<Role>`). Compare with `Role::None` to check if unset.
- **`mb.total_emails()`** returns `usize` directly, not `Option<usize>`.
- **`take_id()` / `take_list()`** require `let mut` on the response object.
- **Filter type inference**: Rust can't infer the generic for `Some(filter.into())` in `email_query()`. Bind to an explicit type: `let filter: core::query::Filter<email::query::Filter> = ...;`
- **`download(blob_id)`** takes only the blob ID — NOT `(account_id, blob_id, name)`.
- **`email_submission_create(email_id, identity_id)`** needs an identity ID, not account ID. Fetch identities via builder pattern.
- **`changes.created()/updated()/destroyed()`** return `&[String]`, not `&[&str]`. Use `.map(String::as_str)` not `.copied()`.
- **`fetch_text_body_values(true)`** is accessed via `get_req.arguments().fetch_text_body_values(true)`, not directly on the get request.
- **`mailbox_changes(since_state, 0)`** — max_changes of 0 is invalid per JMAP spec. Use 500.

---

## Deferred Work

1. **Bearer/OAuth JMAP** — requires per-provider OAuth endpoint config (Fastmail has its own OAuth URLs/scopes), acquisition UI flow, and client rebuild-on-refresh (jmap-client binds credentials at construction). Either rebuild `JmapClientInner` on token refresh, or patch the crate for a credential callback.
2. ~~**Shared Rust `EmailProvider` trait**~~ — **Done.** `ProviderOps` trait in `provider/ops.rs`, implemented by Gmail, JMAP, and Graph providers.
3. ~~**Provider-agnostic Tauri commands**~~ — **Done.** 17 `provider_*` commands in `provider/commands.rs` dispatch via `ProviderOps`.
4. **JMAP push notifications** — `jmap-client` supports WebSocket push (`EventSource`). Could replace polling for real-time sync.
5. **JMAP Sieve filter management** — `jmap-client` supports full Sieve CRUD. Server-side filter management.
6. **List-Unsubscribe** — fetch via `header:List-Unsubscribe:asText` in `Email/get`.
7. **Authentication-Results** — some servers may expose via custom header fetch. Investigate.
8. **JMAP for Calendars** — `jmap-client` has no calendar support (Issue #3).
9. **Server-side pagination for initial sync** — current implementation re-queries full `Email/query` each loop and does client-side `skip(position).take(BATCH_SIZE)`. Should pass JMAP `position` + `limit` parameters for server-side pagination (the TS reference did this correctly).
10. **Batched `Email/set` for actions** — current implementation calls `email_set_mailbox`/`email_set_keyword` per-email sequentially (one API call per email in a thread). Should build a single `Email/set` request with patches for all email IDs in the thread, like the TS reference did.
11. **Batched send request** — current implementation does `email_import()` → `email_submission_create()` → `email_set_keyword()` as 3 sequential calls. Should use a single batched JMAP request with back-references (`Email/import` + `EmailSubmission/set` + `onSuccessUpdateEmail`). The `jmap-client` crate's high-level API doesn't expose this cleanly — may need to use the raw request builder.
12. **`moveToFolder` mailbox removal** — current implementation uses `email_set_mailboxes(vec![target])` which sets exclusive membership. The TS reference fetched each email's current `mailboxIds` and explicitly patched to remove all except target. Verify our approach works correctly against real JMAP servers (Stalwart, Fastmail) — if `email_set_mailboxes` is a jmap-client convenience that does the right thing, this is fine.
13. **Bearer auth client factory pattern** — the TS `clientFactory.ts` checks `account.auth_method === "oauth2" || "bearer"` and passes the `access_token` as Bearer credential. When adding Bearer/OAuth to Rust (item 1), follow this pattern: read `auth_method` from DB, use `Credentials::bearer(token)` instead of `Credentials::basic()`.
14. **`isKnownJmapProvider()` utility** — the TS auto-discovery exports a quick-check function for UI hints (e.g., showing "JMAP supported" badge during account setup). Could add to `auto_discovery.rs` and expose as a Tauri command.
