# IMAP CONDSTORE/QRESYNC (RFC 7162)

**Tier**: 1 - Blocks switching from Outlook
**Status**: ✅ **Phases 1-2 complete, Phase 3 unimplemented (no longer blocked).** The IMAP client primitives live in `crates/imap/`; the sync orchestration that wires them into delta sync lives in `crates/provider-sync/`. Highlights: capability negotiation with iCloud workaround in `crates/imap/src/connection.rs::negotiate_condstore_qresync`; fast-path skip when HIGHESTMODSEQ matches plus reset detection in `crates/imap/src/client/sync.rs::delta_check_folders`; raw `CHANGEDSINCE` FETCH in `crates/imap/src/client/commands.rs::fetch_changed_flags`; sync dispatch in `crates/provider-sync/src/imap/imap_delta.rs`; `modseq` and `last_deletion_check_at` persisted in `folder_sync_state` via `crates/provider-sync/src/imap/sync_pipeline.rs::upsert_folder_sync_state`; UID-based deletion detection and the non-CONDSTORE flag-sync fallback in `crates/provider-sync/src/imap/imap_delta_janitor.rs` (10-min and 5-min throttles). Phase 3 (QRESYNC VANISHED handling) is a feature gap, not a blocker: `imap-proto 0.16.7` already parses `Response::Vanished { earlier, uids }` and `async-imap` re-exports `imap_proto`. The original async-imap issue [chatmail/async-imap#130](https://github.com/chatmail/async-imap/issues/130) was closed `not_planned` after we confirmed CHANGEDSINCE works by appending the modifier to the `uid_fetch` query string.

---

- **What**: Efficient delta sync for IMAP - server tracks mod-sequences, client fetches only changes since last sync
- **Scope**: Stalwart and most modern IMAP servers support this. Critical for users not on Graph/JMAP.

## Pain points

- Capability detection: not all IMAP servers support CONDSTORE/QRESYNC. Need to detect via `CAPABILITY` response and fall back to full UID comparison if absent. The fallback must still work at scale (50k+ messages in a mailbox).
- QRESYNC requires `ENABLE QRESYNC` - must be sent after authentication. Some servers advertise QRESYNC but have buggy implementations. Need defensive handling of malformed `VANISHED` responses.
- Mod-sequence storage: need to persist the highest mod-seq per mailbox per account in the local DB, and handle the case where the server's mod-seq resets (e.g., after a mailbox recreation).
- Interaction with message moves: IMAP doesn't have a native "move" operation pre-RFC 6851 (`MOVE` extension). Without `MOVE`, a copy+delete looks like a new message + an expunge, which complicates delta sync.
- Flagged-only changes: CONDSTORE can report that a message's flags changed without re-downloading the message. Need to handle flag-only updates efficiently (update local DB flags, don't re-fetch body).

## Work

Detect CONDSTORE/QRESYNC capability, implement mod-seq tracking, use `CHANGEDSINCE` in FETCH, handle `VANISHED` for expunges, fall back to UID comparison when unsupported.

## Research

### RFC 7162 deep dive

RFC 7162 consolidates and supersedes RFC 4551 (CONDSTORE) and RFC 5162 (QRESYNC) into a single specification. The two extensions are layered: QRESYNC implies CONDSTORE, and advertising QRESYNC means the server supports everything in CONDSTORE.

#### CONDSTORE mechanics (Section 3)

The core primitive is the **modification sequence** (mod-sequence): a positive unsigned 63-bit value associated with every metadata item (flags, annotations) on every message. The server guarantees that each STORE operation on a mailbox gets a strictly increasing mod-sequence, enabling total ordering of all changes.

**HIGHESTMODSEQ in SELECT/EXAMINE.** When a client issues a CONDSTORE-enabling command (including `SELECT mailbox (CONDSTORE)`), the server returns `OK [HIGHESTMODSEQ <value>]` in the SELECT response. A disconnected client compares its cached HIGHESTMODSEQ against the server's value - if the server's is higher, flag changes have occurred since last sync. If HIGHESTMODSEQ is equal, no flag changes occurred and the client can skip flag resynchronization entirely. This single comparison is the key optimization: for a 50k-message mailbox where nothing changed, CONDSTORE turns flag sync from a full `UID FETCH 1:* (FLAGS)` into a zero-cost no-op.

**FETCH MODSEQ data item.** Clients request per-message mod-sequences via `UID FETCH 1:* (FLAGS MODSEQ)`. The server returns `MODSEQ (<value>)` for each message. After a CONDSTORE-enabling command, the server MUST automatically include MODSEQ in all subsequent untagged FETCH responses for the duration of the connection - including changes caused by external agents (other clients, server-side filters).

**CHANGEDSINCE FETCH modifier.** The key sync primitive: `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <cached-highestmodseq>)` returns ONLY messages whose mod-sequence is greater than the specified value. For a 50k-message mailbox where 3 messages had flag changes, this returns 3 responses instead of 50,000. The server implicitly adds MODSEQ to the response.

**SEARCH MODSEQ criterion.** `UID SEARCH MODSEQ <value>` finds messages with mod-sequence >= the threshold. The server appends `(MODSEQ <highest-matching>)` to non-empty search results. This is an alternative to FETCH CHANGEDSINCE when the client only needs UIDs of changed messages, not their flags.

**STORE UNCHANGEDSINCE modifier.** `UID STORE <set> (UNCHANGEDSINCE <modseq>) +FLAGS (\Seen)` performs a conditional store - the server only applies the change if the message's current mod-sequence is <= the specified value. On conflict, the server returns `[MODIFIED <uid-set>]` listing UIDs that failed. This prevents lost-update races when multiple clients modify flags concurrently.

**CONDSTORE activation.** CONDSTORE mode activates implicitly when a client uses any CONDSTORE command (SELECT CONDSTORE, FETCH CHANGEDSINCE, STORE UNCHANGEDSINCE, SEARCH MODSEQ). Once activated, the server includes MODSEQ in all FETCH responses for the rest of the session. No explicit ENABLE is needed for CONDSTORE alone.

#### QRESYNC mechanics (Section 4)

QRESYNC extends CONDSTORE to handle message expunges in addition to flag changes, enabling full resync in a single round trip.

**ENABLE QRESYNC.** Must be sent after authentication, before SELECT. The server responds with an untagged `ENABLED QRESYNC` response. Once enabled, the server MUST send `VANISHED` responses instead of `EXPUNGE` responses for the rest of the session. A server MUST reject QRESYNC SELECT parameters and VANISHED FETCH modifiers if `ENABLE QRESYNC` was not issued first.

**SELECT QRESYNC parameter.** Syntax: `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>] [(<seq-set> <uid-set>)]))`

The server processes this as:
1. Validates UIDVALIDITY - if the client's cached value doesn't match, the server ignores remaining QRESYNC parameters and returns a normal SELECT response (signaling full resync needed).
2. If UIDVALIDITY matches, sends untagged FETCH responses for all messages with mod-sequence > the client's cached value (flag changes).
3. Sends `VANISHED (EARLIER) <uid-set>` listing all UIDs that have been expunged since the client's cached mod-sequence.
4. The optional `<known-uids>` parameter lets the client tell the server which UIDs it has cached, so the server only reports relevant expunges.

This collapses what would otherwise require SELECT + UID FETCH CHANGEDSINCE + UID SEARCH for expunge detection into a single command-response exchange.

**VANISHED response types.** Two forms:
- `* VANISHED (EARLIER) <uid-set>` - sent during SELECT QRESYNC or UID FETCH VANISHED. Does NOT decrement message count or adjust sequence numbers. These are historical expunges the client missed.
- `* VANISHED <uid-set>` - sent during normal operation (replaces EXPUNGE after ENABLE QRESYNC). DOES decrement message count and adjusts sequence numbers. These are real-time expunges.

**VANISHED UID FETCH modifier.** `UID FETCH <set> (FLAGS) (CHANGEDSINCE <modseq> VANISHED)` combines flag fetching with expunge reporting. The server returns VANISHED (EARLIER) for UIDs in the set that no longer exist, and FETCH responses for UIDs that changed.

#### UIDVALIDITY interaction

UIDVALIDITY is the guard rail for the entire system. When UIDVALIDITY changes (mailbox recreated, server database rebuilt), all cached UIDs and mod-sequences are invalid. The client MUST:
1. Delete the cached HIGHESTMODSEQ value
2. Discard all cached UID-to-message mappings for that mailbox
3. Perform a full initial sync

In the QRESYNC SELECT flow, if the client sends a stale UIDVALIDITY, the server silently ignores the QRESYNC parameters and returns a fresh SELECT response with the new UIDVALIDITY. The client detects this by comparing UIDVALIDITY values and triggers a full resync.

Our codebase already handles UIDVALIDITY changes in `crates/imap/src/imap_delta.rs` (`process_folder_delta` triggers full resync when `delta.uidvalidity_changed` is true). This logic remains valid for CONDSTORE - we just need to additionally clear the cached HIGHESTMODSEQ.

### Current codebase state

CONDSTORE is fully implemented (Phases 1-2). The IMAP client primitives live in `crates/imap/`; the sync orchestration that drives them lives in `crates/provider-sync/`; DB schema and `DbFolderSyncState` come from `crates/db/`.

**Foundation:**
- `ImapFolderStatus` has `highest_modseq: Option<u64>` (`crates/imap/src/types.rs`)
- `async-imap` parses `HIGHESTMODSEQ` from SELECT responses into `Mailbox.highest_modseq`
- The `folder_sync_state` table has `modseq INTEGER` and `last_deletion_check_at INTEGER` columns (schema in `crates/db/src/db/schema/10_sync.sql`)
- `ImapFolderStatus` is populated with `highest_modseq` on every SELECT across `crates/imap/src/client/`
- `DbFolderSyncState` in `crates/db/src/db/types.rs` carries `modseq: Option<i64>` and `last_deletion_check_at: Option<i64>`

**What's been implemented (Phases 1-2 complete):**
- Capability detection and QRESYNC negotiation: `crates/imap/src/connection.rs::negotiate_condstore_qresync` probes CAPABILITY for `CONDSTORE`/`QRESYNC`, sends `ENABLE QRESYNC` when advertised, watches for the `ENABLED QRESYNC` reply, and falls back to CONDSTORE-only when the server (iCloud) omits it. Session-scoped state is `CondstoreQresyncState { condstore: bool, qresync: bool }`.
- CONDSTORE fast-path: `crates/imap/src/client/sync.rs::delta_check_folders` compares cached modseq vs server `HIGHESTMODSEQ`, skips UID SEARCH when unchanged, and flags modseq reset (server < cached at the same UIDVALIDITY) so the caller triggers a full flag resync.
- CHANGEDSINCE FETCH: `crates/imap/src/client/commands.rs::fetch_changed_flags` issues `UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE <modseq>)` by appending the modifier to the `uid_fetch` query string - the workaround validated in chatmail/async-imap#130.
- Sync dispatch: `crates/provider-sync/src/imap/imap_delta.rs::process_folder_delta` calls `fetch_changed_flags` when modseq changed, then writes via `apply_flag_changes` and `upsert_folder_sync_state` in `crates/provider-sync/src/imap/sync_pipeline.rs`.
- Non-CONDSTORE fallback: `crates/provider-sync/src/imap/imap_delta_janitor.rs::sync_flags_without_condstore` runs periodic full `UID FETCH 1:* (UID FLAGS)` for Exchange IMAP, Courier, hMailServer, etc., throttled by `FLAG_SYNC_INTERVAL_SECS = 300`.
- UID-based deletion detection: `crates/provider-sync/src/imap/imap_delta_janitor.rs::run_deletion_detection` runs `UID SEARCH ALL` diffed against locally cached UIDs, throttled by `DELETION_CHECK_INTERVAL_SECS = 600` via the `last_deletion_check_at` column.

**Remaining gap (Phase 3):** QRESYNC VANISHED handling is unimplemented but no longer blocked. `imap-proto 0.16.7` (re-exported as `async_imap::imap_proto`) parses `* VANISHED [(EARLIER)] <uid-set>` into `Response::Vanished { earlier: bool, uids: Vec<UidSetMember> }`. Nothing in our code currently consumes that variant: the QRESYNC capability is negotiated but never used to drive a `SELECT mailbox (QRESYNC (...))` or a `UID FETCH ... VANISHED` command. The original async-imap issue ([chatmail/async-imap#130](https://github.com/chatmail/async-imap/issues/130)) was closed `not_planned` - it was about CHANGEDSINCE, and that ships today. Per-message `MODSEQ` in FETCH responses is still not parsed by imap-proto, but the mailbox-level `HIGHESTMODSEQ` from SELECT is sufficient for the QRESYNC flow.

### Rust IMAP crate CONDSTORE support

#### async-imap (current - v0.11)

**Supported:**
- `select_condstore()` method - sends `SELECT mailbox (CONDSTORE)`, returns `Mailbox` with `highest_modseq: Option<u64>`. This is a proper first-class API.
- `run_command()` / `run_command_and_check_ok()` / `run_command_untagged()` - raw command execution for anything the typed API doesn't cover.
- `Mailbox.highest_modseq` - parsed from SELECT OK responses by `imap-proto`.

**Not surfaced as typed API (use raw command + response matching):**
- `UID FETCH ... (CHANGEDSINCE ...)` - no typed modifier, but as of chatmail/async-imap#130 the practical pattern is to append `(CHANGEDSINCE <modseq>)` to the `uid_fetch` query string. The typed `Fetch` stream is fine for consuming the response.
- `ENABLE QRESYNC` - no typed method; must use `run_command("ENABLE QRESYNC")` and watch for `ENABLED QRESYNC` in the response stream.
- `SELECT mailbox (QRESYNC (...))` - no typed method; must construct the raw command string.
- `UID FETCH ... (CHANGEDSINCE <m> VANISHED)` - no typed modifier; construct the raw query string.

**Supported via the re-exported `imap-proto`:**
- `* VANISHED [(EARLIER)] <uid-set>` response parsing - `imap-proto 0.16.7` parses these into `Response::Vanished { earlier: bool, uids: Vec<UidSetMember> }`. `async-imap` does `pub use imap_proto;`, so consumers can pattern-match the variant directly off the response stream after a raw `run_command`. (Earlier doc revisions claimed this was unsupported - that's no longer true.)
- `HIGHESTMODSEQ` in SELECT OK responses - parsed into `Mailbox.highest_modseq`.

**Still not parsed:**
- `MODSEQ` data item in FETCH responses - imap-proto does not surface the per-message mod-sequence. Not needed for our Phase 3 path (we only need the mailbox-level HIGHESTMODSEQ and the VANISHED UID list), but would block hypothetical `STORE UNCHANGEDSINCE` conflict resolution.
- `STORE UNCHANGEDSINCE` - no typed modifier and no per-message MODSEQ to compare against.

**Practical approach with async-imap:** the same hybrid pattern works for all three phases - typed API where it exists (`select`, `uid_fetch`), raw-command + response matching where it doesn't (`ENABLE QRESYNC`, `SELECT (QRESYNC (...))`, `VANISHED` responses). This is the same pattern Delta Chat uses and is consistent with our existing `crates/imap/src/raw.rs` fallback path.

#### imap-codec / imap-types (duesee - v2.0.0-alpha)

The `imap-types` crate has an `ext_condstore_qresync` feature flag. However, as of March 2026, this feature is explicitly marked **"Unfinished"** in the documentation. The feature flag exists and exposes partial type definitions, but the parser and serializer coverage is incomplete.

Combined with the Gmail SELECT crash (Himalaya issue #641) and the broader maturity issues documented in `docs/imap-ecosystem-assessment.md`, `imap-codec` is not a viable path for CONDSTORE implementation today. If the duesee project matures and completes the `ext_condstore_qresync` feature, it would be the architecturally correct solution - proper type-safe CONDSTORE/QRESYNC with fuzz-tested parsing. But that's a speculative future, not a present option.

### Server support matrix

| Server | CONDSTORE | QRESYNC | MOVE (RFC 6851) | Notes |
|--------|-----------|---------|-----------------|-------|
| **Dovecot** (2.x+) | Yes | Yes | Yes | Both enabled by default. Most widely deployed IMAP server. Gold standard implementation. |
| **Cyrus** (3.x+) | Yes | Yes | Yes | Powers Fastmail. Full RFC 7162 compliance. |
| **Stalwart** | Yes | Yes | Yes | Written in Rust. Full IMAP4rev2 support including mandatory CONDSTORE. Maintains mod-sequence changelog for QRESYNC. |
| **Gmail IMAP** | Yes | **No** | Yes | CONDSTORE only. The only major provider that supports CONDSTORE without QRESYNC. Non-standard quirks (see Gotchas below). |
| **iCloud** | Yes | Yes (buggy) | Yes | Advertises both, but implementation has issues: doesn't send required ENABLED response, produces invalid FETCH responses (negative sequence numbers) during QRESYNC SELECT. |
| **Yahoo/AOL** | Yes | Yes | Yes | Full support. CONDSTORE and OBJECTID available on all mailboxes. |
| **Exchange/O365 IMAP** | No | No | Yes | Exchange IMAP is a limited compatibility layer. No CONDSTORE, no QRESYNC. Microsoft pushes Graph API instead. |
| **Zimbra** | Yes | Yes | Yes | Full RFC 7162 support. Tested by Thunderbird. |
| **Courier IMAP** | No | No | No | No CONDSTORE/QRESYNC. No MOVE. Maintainer has cited implementation complexity as the blocker. Legacy server, declining usage. |
| **hMailServer** | No | No | Limited | Windows-only, minimal extension support. No CONDSTORE/QRESYNC. |
| **Postfix + Dovecot** | Yes | Yes | Yes | Common Linux combo; IMAP capability comes from Dovecot. |

**Key takeaway:** CONDSTORE is available on all the servers that matter for Ratatoskr's target users (Dovecot, Cyrus, Gmail, Yahoo, iCloud). QRESYNC is available on all of those except Gmail. Exchange IMAP lacks both, but we already have a Microsoft Graph provider that handles Exchange/O365 with proper delta sync. The servers that lack CONDSTORE (Courier, hMailServer) are legacy/niche and need the UID-comparison fallback regardless.

### Implementation patterns in other clients

#### Delta Chat (async-imap, Rust)

Delta Chat (chatmail/core, formerly deltachat-core-rust) uses the same `async-imap` crate we do. Their CONDSTORE approach (documented in [issue #2941](https://github.com/deltachat/deltachat-core-rust/issues/2941)):

1. Check server CAPABILITY for `CONDSTORE`.
2. If supported, use `SELECT ... (CONDSTORE)` to get HIGHESTMODSEQ.
3. Store HIGHESTMODSEQ in their sync state table (analogous to our `folder_sync_state.modseq`).
4. On reconnect, if server's HIGHESTMODSEQ > cached value, issue `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <cached>)` via raw command.
5. They explicitly chose NOT to implement QRESYNC - "Since Delta Chat is not interested in expunged messages, for better compatibility it is enough to support CONDSTORE extension."

Delta Chat's scope is narrower than ours (they mainly care about `\Seen` and `$MDNSent` flag sync across devices), but their async-imap integration pattern is directly applicable: typed API for SELECT, raw commands for CHANGEDSINCE, skip QRESYNC.

#### Thunderbird (C++, desktop)

Thunderbird has had CONDSTORE support since approximately 2009, but it has been a long source of bugs:
- [Bug 912216](https://bugzilla.mozilla.org/show_bug.cgi?id=912216): CONDSTORE was disabled by default for years due to interaction bugs with IDLE and expunge notifications.
- [Bug 1124569](https://bugzilla.mozilla.org/show_bug.cgi?id=1124569): When CONDSTORE is used without IDLE, expunged messages aren't removed from the local database.
- [Bug 1123094](https://bugzilla.mozilla.org/show_bug.cgi?id=1123094): Folder contents may not be correct with CONDSTORE enabled.
- [Bug 1747311](https://bugzilla.mozilla.org/show_bug.cgi?id=1747311): QRESYNC implementation landed in 2022-2025 timeframe, described as "large and complicated" requiring extensive testing with various server types.

Thunderbird's experience is a cautionary tale: CONDSTORE looks simple in the RFC but has a long tail of interaction bugs, especially around expunge handling and IDLE notifications. Their QRESYNC work took years.

#### Thunderbird Android (K-9 Mail, Kotlin)

[PR #2607](https://github.com/thunderbird/thunderbird-android/pull/2607) implements CONDSTORE/QRESYNC for the Android client. The implementation pattern:
1. Detect capabilities.
2. Store HIGHESTMODSEQ per folder.
3. On sync: if QRESYNC supported, use SELECT QRESYNC to get flag changes + VANISHED in one round trip.
4. If only CONDSTORE: use UID FETCH CHANGEDSINCE for flags, then UID SEARCH to detect deletes.
5. Fallback: full UID comparison.

This three-tier approach (QRESYNC > CONDSTORE > UID comparison) is the canonical pattern.

### MOVE extension interaction (RFC 6851)

MOVE is relevant because it interacts with CONDSTORE/QRESYNC mod-sequence tracking:

- When a server executes MOVE/UID MOVE, it MUST increment the per-mailbox mod-sequence and send an updated `HIGHESTMODSEQ` in the response.
- With QRESYNC enabled, the server sends `VANISHED` (not `EXPUNGE`) for moved messages in the source mailbox.
- Servers supporting UIDPLUS SHOULD send `COPYUID` in the MOVE response, giving the client the new UIDs in the destination mailbox.
- Without MOVE (copy+delete+expunge), the source mailbox sees a flag change (`\Deleted`) followed by expunge, and the destination sees a new message. With CONDSTORE, both changes are tracked by mod-sequence. Without CONDSTORE, the client must detect both independently.

Our `move_messages()` in `crates/imap/src/client/commands.rs` already tries MOVE first and falls back to COPY+DELETE+EXPUNGE. This is correct for CONDSTORE - the mod-sequence increments will be captured by CHANGEDSINCE on next sync regardless of which path was taken.

### IDLE interaction

The RFC does not mandate that IDLE notifications include MODSEQ data. In practice:

- After a CONDSTORE-enabling command, the server MUST include MODSEQ in all untagged FETCH responses, including those generated during IDLE. So if another client changes a flag while we're in IDLE, we'll get a FETCH response with MODSEQ.
- However, EXPUNGE notifications during IDLE do NOT include MODSEQ. With QRESYNC enabled, the server sends VANISHED instead of EXPUNGE, which includes UIDs (but still not MODSEQ).
- EXISTS notifications (new messages) during IDLE never include MODSEQ. The client must issue a FETCH after leaving IDLE to get the new message's metadata.

**Practical impact for Ratatoskr:** Our IDLE handler currently exits IDLE and does a delta check on any notification. With CONDSTORE, the delta check uses CHANGEDSINCE instead of UID SEARCH for flag changes. This is already implemented in `crates/imap/src/imap_delta.rs`.

### Data model for mod-seq tracking

The schema is in place (defined in `crates/db/src/db/migrations.rs`). The `folder_sync_state` table has:

```sql
CREATE TABLE folder_sync_state (
  account_id TEXT NOT NULL,
  folder_path TEXT NOT NULL,
  uidvalidity INTEGER,
  last_uid INTEGER DEFAULT 0,
  modseq INTEGER,                -- ← populated with server's HIGHESTMODSEQ
  last_sync_at INTEGER,
  last_deletion_check_at INTEGER, -- ← throttles UID-based deletion detection
  PRIMARY KEY (account_id, folder_path)
);
```

**Status of these changes (all complete):**

1. **modseq is written.** `upsert_folder_sync_state()` in `crates/imap/src/sync_pipeline.rs` accepts `modseq: Option<u64>` and writes the server's HIGHESTMODSEQ from SELECT responses.

2. **`modseq` field is active.** `FolderSyncState` in `crates/imap/src/sync_pipeline.rs` has `modseq: Option<u64>` (actively read and used in delta sync logic). Only `_last_sync_at` remains underscore-prefixed.

3. **Capability state is per-session.** `CondstoreQresyncState` in `crates/imap/src/connection.rs` tracks `condstore: bool` and `qresync: bool`, negotiated via `negotiate_condstore_qresync()` after authentication.

4. **No new tables needed.** Per-message mod-sequences are not stored locally - only the mailbox-level HIGHESTMODSEQ for CHANGEDSINCE queries. A `last_deletion_check_at` column was added to `folder_sync_state` for throttling UID-based deletion detection.

### Fallback strategy for servers without CONDSTORE

For servers without CONDSTORE (Exchange IMAP via non-Graph path, Courier, hMailServer, miscellaneous corporate servers), the current UID-based approach must remain:

**Current fallback implementation (in `crates/imap/src/imap_delta.rs`):**
1. **New messages:** `UID SEARCH last_uid+1:*` to find new messages.
2. **UIDVALIDITY comparison** to detect mailbox recreation.
3. **Flag changes:** `sync_flags_without_condstore()` performs periodic `UID FETCH 1:* (FLAGS)` and diffs against local DB. For a 50k-message mailbox this returns ~50k small responses (UID + flags only, no bodies). Expensive but unavoidable without CONDSTORE. Throttled via `NON_CONDSTORE_FLAG_SYNC_INTERVAL_SECS`.
4. **Deletions:** `run_deletion_detection()` performs `UID SEARCH ALL`, diffs against locally cached UIDs. Throttled to 10-minute intervals per folder via `last_deletion_check_at`.

This is the same approach every IMAP client without CONDSTORE uses. The cost is O(N) per folder where N = message count, vs O(delta) with CONDSTORE.

### Practical gotchas

**Gmail IMAP CONDSTORE quirks:**
- Gmail supports CONDSTORE but NOT QRESYNC. It is reportedly the only major provider in this configuration.
- Thunderbird encountered multiple bugs with Gmail's CONDSTORE: new mail notifications not showing when CONDSTORE is active ([Bug 885220](https://bugzilla.mozilla.org/show_bug.cgi?id=885220)), EXPUNGE responses being lost when CONDSTORE is used without IDLE ([Bug 1124569](https://bugzilla.mozilla.org/show_bug.cgi?id=1124569)).
- Gmail may not consistently report `HIGHESTMODSEQ` in all SELECT responses. Some clients have observed `CONDSTORE` in the capability list but no `HIGHESTMODSEQ` in the SELECT response, which per the RFC means the server doesn't support persistent mod-sequences for that mailbox.
- Since Ratatoskr has a dedicated Gmail API provider, Gmail IMAP CONDSTORE is lower priority - but it matters for users who configure Gmail via generic IMAP rather than the Gmail API path.

**iCloud IMAP QRESYNC quirks:**
- iCloud advertises QRESYNC but doesn't send the required `ENABLED` untagged response after `ENABLE QRESYNC`.
- iCloud produces invalid FETCH responses during QRESYNC SELECT, including negative sequence numbers.
- Defensive approach: after ENABLE QRESYNC, verify the ENABLED response was received. If not, fall back to CONDSTORE-only mode.

**Dovecot:**
- Gold standard implementation. Both CONDSTORE and QRESYNC work as specified.
- QRESYNC is enabled by default since Dovecot 2.2+.
- Dovecot's VANISHED responses are well-formed. Use Dovecot as the reference server for testing.

**CONDSTORE without HIGHESTMODSEQ:**
- Per RFC 7162, a server that doesn't return HIGHESTMODSEQ in SELECT does not support persistent mod-sequences for that mailbox, even if CONDSTORE is in the capability list. Must check for HIGHESTMODSEQ presence, not just CONDSTORE capability.
- Our `ImapFolderStatus.highest_modseq` (in `crates/imap/src/types.rs`) is already `Option<u64>`, so this check is natural: if `highest_modseq.is_none()`, fall back to UID comparison.

**Mod-sequence resets:**
- A server MAY reset mod-sequences, in which case UIDVALIDITY will also change. Our existing UIDVALIDITY change detection handles this automatically.
- The degenerate case: UIDVALIDITY unchanged but mod-sequences reset. This violates the RFC but could happen with buggy servers. Defense: if HIGHESTMODSEQ < cached value and UIDVALIDITY is unchanged, treat as a reset and do full resync.

### Recommended implementation plan

**Phase 1 - CONDSTORE flag sync: COMPLETE.**
Implemented in `crates/imap/src/connection.rs` (capability detection, QRESYNC negotiation), `crates/imap/src/client/sync.rs` (CONDSTORE fast-path in `delta_check_folders`), `crates/imap/src/client/commands.rs` (`fetch_flags_changedsince`), `crates/imap/src/imap_delta.rs` (CHANGEDSINCE flag sync in `process_folder_delta`), and `crates/imap/src/sync_pipeline.rs` (modseq persistence, `apply_flag_changes`). Non-CONDSTORE fallback via `sync_flags_without_condstore()` handles servers without CONDSTORE support.

**Phase 2 - Deletion detection: COMPLETE.**
UID-based deletion detection without QRESYNC is implemented in `crates/imap/src/imap_delta.rs::run_deletion_detection`. Uses `UID SEARCH ALL` diffed against locally cached UIDs, throttled per-folder (10-minute interval via `last_deletion_check_at` column in `folder_sync_state`).

**Phase 3 - QRESYNC VANISHED handling: NOT BLOCKED, NOT YET IMPLEMENTED.**

QRESYNC negotiation is implemented (`ENABLE QRESYNC` with iCloud workaround). `imap-proto 0.16.7` parses `Response::Vanished { earlier, uids }`. `async-imap` re-exports `imap_proto`. The original blocker is gone; what's left is wiring it up. The Thunderbird history above (years of CONDSTORE/IDLE/expunge bugs, QRESYNC described as "large and complicated") and the iCloud QRESYNC defects mean Phase 3 is high-risk-of-regression. Treat it accordingly:

**Wiring (the easy part):**

1. After `negotiate_condstore_qresync` returns `qresync: true`, replace the plain SELECT in `delta_check_folders` with a raw `SELECT mailbox (QRESYNC (<uidvalidity> <last-modseq>))` via `Session::run_command`. Cached UIDVALIDITY and modseq already live in `folder_sync_state`.
2. Drain the response stream after the SELECT (and after `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <modseq> VANISHED)` if used as the post-SELECT FETCH form). Pattern-match `async_imap::imap_proto::Response::Vanished { earlier, uids }` and remove the listed UIDs from the local store. UIDs are stable, so for our purposes `EARLIER` and non-`EARLIER` VANISHED are equivalent - we don't track sequence numbers.
3. When `CondstoreQresyncState.qresync` is true, replace the throttled `run_deletion_detection` UID-comparison path with the QRESYNC results. Keep `run_deletion_detection` as the fallback for CONDSTORE-only servers (notably Gmail IMAP) and servers with neither extension.

**Footguns we must defend against (lessons from `### Implementation patterns in other clients`):**

- **VANISHED-without-IDLE drops (Thunderbird Bug 1124569).** When CONDSTORE was used without IDLE, expunged messages stuck around in the local DB. Root cause: real-time `* VANISHED <uids>` arrives untagged at arbitrary points in the response stream, and code paths that only checked SELECT-time responses missed them. Mitigation: every code path that issues an IMAP command on a QRESYNC-enabled session must drain untagged responses and feed any `Response::Vanished` (earlier or not) through the deletion path - not just SELECT and FETCH.

- **Folder contents drift (Thunderbird Bug 1123094).** Cache-coherence bugs surface when VANISHED is applied but a subsequent EXISTS bump or new-UID FETCH lands without proper bookkeeping. Mitigation: after applying VANISHED + any new FETCHes for a folder, cross-check local UID count against `Mailbox.exists`. On mismatch, fall back to a full `UID SEARCH ALL` reconciliation rather than trusting the QRESYNC delta. Log the divergence loudly - this is a server bug, our bug, or both.

- **Gmail CONDSTORE flakes (Thunderbird Bug 885220).** Gmail doesn't advertise QRESYNC, so Phase 3 doesn't touch it, but the related lesson is "advertised capability + missing response data" is a real failure mode. Treat `mailbox.highest_modseq.is_none()` as "this folder doesn't support persistent mod-sequences here" and skip CONDSTORE for that folder even if the capability was negotiated - already correct in `delta_check_folders`, must remain correct after the QRESYNC SELECT switch.

- **iCloud's "advertised but broken" QRESYNC.** Two known defects: (a) no `ENABLED QRESYNC` after `ENABLE` (already handled - we drop to CONDSTORE-only), (b) negative sequence numbers in FETCH responses during QRESYNC SELECT. Mitigation for (b): if we somehow ENABLE'd against a buggy server, parse failures or out-of-range sequence numbers during the response drain must be treated as a one-shot signal to disable QRESYNC for that session (clear `qresync`, fall back to CONDSTORE-only) rather than retried.

- **Mod-sequence reset without UIDVALIDITY change.** Already detected in `delta_check_folders` via `modseq_reset` (server modseq < cached at same UIDVALIDITY). The QRESYNC SELECT must preserve this: if the server's response carries a HIGHESTMODSEQ lower than our cached value, abandon the QRESYNC parameters and do a full resync.

- **TB's "years to ship" pattern.** Land Phase 3 behind a runtime gate (off by default), validate against Dovecot (gold standard), Stalwart (Rust, RFC-strict), and Cyrus before flipping the default. Keep `run_deletion_detection` reachable as a panic-button fallback per-account, not just per-capability.

- **Delta Chat's deliberate punt.** Delta Chat uses the same async-imap stack we do and explicitly chose not to implement QRESYNC, on the grounds that compatibility risk wasn't worth the upside for their use case (they don't care about expunged messages). Their stance is the strongest "you can stop at Phase 2" argument we have. Our 150 GB cached-mailbox / multi-year history requirement is the reason we don't take that exit - but if Phase 3 keeps regressing in field testing, CONDSTORE-only + `run_deletion_detection` indefinitely is a viable end state, not a failure. Decide that consciously rather than discovering it by attrition.

Per-message `MODSEQ` in FETCH is still unparsed by imap-proto; that doesn't block Phase 3 but would block `STORE UNCHANGEDSINCE` lost-update protection.

### Sources

- [RFC 7162: IMAP Extensions: CONDSTORE and QRESYNC](https://datatracker.ietf.org/doc/html/rfc7162)
- [RFC 6851: IMAP MOVE Extension](https://www.rfc-editor.org/rfc/rfc6851.html)
- [async-imap Session API (docs.rs)](https://docs.rs/async-imap/latest/async_imap/struct.Session.html)
- [async-imap Mailbox struct (docs.rs)](https://docs.rs/async-imap/latest/async_imap/types/struct.Mailbox.html)
- [imap-codec GitHub (duesee)](https://github.com/duesee/imap-codec)
- [imap-types docs.rs](https://docs.rs/imap-types/latest/imap_types/)
- [Delta Chat CONDSTORE issue #2941](https://github.com/deltachat/deltachat-core-rust/issues/2941)
- [Delta Chat CONDSTORE/QRESYNC issue #200](https://github.com/deltachat/deltachat-core/issues/200)
- [chatmail/async-imap#130 - CHANGEDSINCE typed support](https://github.com/chatmail/async-imap/issues/130) (closed `not_planned`: the conclusion was to append `CHANGEDSINCE` to the `uid_fetch` query string)
- [Thunderbird QRESYNC Bug 1747311](https://bugzilla.mozilla.org/show_bug.cgi?id=1747311)
- [Thunderbird CONDSTORE Bug 912216](https://bugzilla.mozilla.org/show_bug.cgi?id=912216)
- [Thunderbird Android CONDSTORE/QRESYNC PR #2607](https://github.com/thunderbird/thunderbird-android/pull/2607)
- [iCloud QRESYNC discussion (Apple Developer Forums)](https://developer.apple.com/forums/thread/694251)
- [Stalwart Mail Server RFCs](https://stalw.art/docs/development/rfcs/)
- [MailCore2 Gmail CONDSTORE issue #297](https://github.com/MailCore/mailcore2/issues/297)
- [Cyrus IMAP RFC Support](https://www.cyrusimap.org/3.10/imap/rfc-support.html)
