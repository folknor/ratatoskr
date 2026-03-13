# IMAP CONDSTORE/QRESYNC (RFC 7162)

**Tier**: 1 — Blocks switching from Outlook
**Status**: ❌ **Not implemented** — The `modseq` column exists in `folder_sync_state` and `ImapFolderStatus` parses it from SELECT responses, but it's unused (`_modseq`). Sync relies on UID comparison only. No capability detection, no `CHANGEDSINCE`/`VANISHED` handling.

---

- **What**: Efficient delta sync for IMAP — server tracks mod-sequences, client fetches only changes since last sync
- **Scope**: Stalwart and most modern IMAP servers support this. Critical for users not on Graph/JMAP.

## Pain points

- Capability detection: not all IMAP servers support CONDSTORE/QRESYNC. Need to detect via `CAPABILITY` response and fall back to full UID comparison if absent. The fallback must still work at scale (50k+ messages in a mailbox).
- QRESYNC requires `ENABLE QRESYNC` — must be sent after authentication. Some servers advertise QRESYNC but have buggy implementations. Need defensive handling of malformed `VANISHED` responses.
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

**HIGHESTMODSEQ in SELECT/EXAMINE.** When a client issues a CONDSTORE-enabling command (including `SELECT mailbox (CONDSTORE)`), the server returns `OK [HIGHESTMODSEQ <value>]` in the SELECT response. A disconnected client compares its cached HIGHESTMODSEQ against the server's value — if the server's is higher, flag changes have occurred since last sync. If HIGHESTMODSEQ is equal, no flag changes occurred and the client can skip flag resynchronization entirely. This single comparison is the key optimization: for a 50k-message mailbox where nothing changed, CONDSTORE turns flag sync from a full `UID FETCH 1:* (FLAGS)` into a zero-cost no-op.

**FETCH MODSEQ data item.** Clients request per-message mod-sequences via `UID FETCH 1:* (FLAGS MODSEQ)`. The server returns `MODSEQ (<value>)` for each message. After a CONDSTORE-enabling command, the server MUST automatically include MODSEQ in all subsequent untagged FETCH responses for the duration of the connection — including changes caused by external agents (other clients, server-side filters).

**CHANGEDSINCE FETCH modifier.** The key sync primitive: `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <cached-highestmodseq>)` returns ONLY messages whose mod-sequence is greater than the specified value. For a 50k-message mailbox where 3 messages had flag changes, this returns 3 responses instead of 50,000. The server implicitly adds MODSEQ to the response.

**SEARCH MODSEQ criterion.** `UID SEARCH MODSEQ <value>` finds messages with mod-sequence >= the threshold. The server appends `(MODSEQ <highest-matching>)` to non-empty search results. This is an alternative to FETCH CHANGEDSINCE when the client only needs UIDs of changed messages, not their flags.

**STORE UNCHANGEDSINCE modifier.** `UID STORE <set> (UNCHANGEDSINCE <modseq>) +FLAGS (\Seen)` performs a conditional store — the server only applies the change if the message's current mod-sequence is <= the specified value. On conflict, the server returns `[MODIFIED <uid-set>]` listing UIDs that failed. This prevents lost-update races when multiple clients modify flags concurrently.

**CONDSTORE activation.** CONDSTORE mode activates implicitly when a client uses any CONDSTORE command (SELECT CONDSTORE, FETCH CHANGEDSINCE, STORE UNCHANGEDSINCE, SEARCH MODSEQ). Once activated, the server includes MODSEQ in all FETCH responses for the rest of the session. No explicit ENABLE is needed for CONDSTORE alone.

#### QRESYNC mechanics (Section 4)

QRESYNC extends CONDSTORE to handle message expunges in addition to flag changes, enabling full resync in a single round trip.

**ENABLE QRESYNC.** Must be sent after authentication, before SELECT. The server responds with an untagged `ENABLED QRESYNC` response. Once enabled, the server MUST send `VANISHED` responses instead of `EXPUNGE` responses for the rest of the session. A server MUST reject QRESYNC SELECT parameters and VANISHED FETCH modifiers if `ENABLE QRESYNC` was not issued first.

**SELECT QRESYNC parameter.** Syntax: `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>] [(<seq-set> <uid-set>)]))`

The server processes this as:
1. Validates UIDVALIDITY — if the client's cached value doesn't match, the server ignores remaining QRESYNC parameters and returns a normal SELECT response (signaling full resync needed).
2. If UIDVALIDITY matches, sends untagged FETCH responses for all messages with mod-sequence > the client's cached value (flag changes).
3. Sends `VANISHED (EARLIER) <uid-set>` listing all UIDs that have been expunged since the client's cached mod-sequence.
4. The optional `<known-uids>` parameter lets the client tell the server which UIDs it has cached, so the server only reports relevant expunges.

This collapses what would otherwise require SELECT + UID FETCH CHANGEDSINCE + UID SEARCH for expunge detection into a single command-response exchange.

**VANISHED response types.** Two forms:
- `* VANISHED (EARLIER) <uid-set>` — sent during SELECT QRESYNC or UID FETCH VANISHED. Does NOT decrement message count or adjust sequence numbers. These are historical expunges the client missed.
- `* VANISHED <uid-set>` — sent during normal operation (replaces EXPUNGE after ENABLE QRESYNC). DOES decrement message count and adjusts sequence numbers. These are real-time expunges.

**VANISHED UID FETCH modifier.** `UID FETCH <set> (FLAGS) (CHANGEDSINCE <modseq> VANISHED)` combines flag fetching with expunge reporting. The server returns VANISHED (EARLIER) for UIDs in the set that no longer exist, and FETCH responses for UIDs that changed.

#### UIDVALIDITY interaction

UIDVALIDITY is the guard rail for the entire system. When UIDVALIDITY changes (mailbox recreated, server database rebuilt), all cached UIDs and mod-sequences are invalid. The client MUST:
1. Delete the cached HIGHESTMODSEQ value
2. Discard all cached UID-to-message mappings for that mailbox
3. Perform a full initial sync

In the QRESYNC SELECT flow, if the client sends a stale UIDVALIDITY, the server silently ignores the QRESYNC parameters and returns a fresh SELECT response with the new UIDVALIDITY. The client detects this by comparing UIDVALIDITY values and triggers a full resync.

Our codebase already handles UIDVALIDITY changes in `imap_delta.rs` (`process_folder_delta` triggers full resync when `delta.uidvalidity_changed` is true). This logic remains valid for CONDSTORE — we just need to additionally clear the cached HIGHESTMODSEQ.

### Current codebase state

The groundwork for CONDSTORE exists but is incomplete:

**What's already in place:**
- `ImapFolderStatus` struct has `highest_modseq: Option<u64>` (in `core/src/imap/types.rs`)
- `async-imap` parses `HIGHESTMODSEQ` from SELECT responses into `Mailbox.highest_modseq`
- The `folder_sync_state` table has a `modseq INTEGER` column (migration v14 in `core/src/db/migrations.rs`)
- `ImapFolderStatus` is populated with `highest_modseq` on every SELECT in `client.rs` (lines 105, 447, 664, 723)
- The DB types (`DbFolderSyncState`) include `modseq: Option<i64>`

**What's not connected:**
- `FolderSyncState` in `pipeline.rs` stores the modseq column as `_modseq` (underscore prefix = unused)
- `upsert_folder_sync_state()` hardcodes `modseq = NULL` on every write
- No CONDSTORE capability detection — `CAPABILITY` response is never inspected for `CONDSTORE` or `QRESYNC`
- No call to `select_condstore()` — all SELECTs use plain `session.select()`
- No CHANGEDSINCE FETCH — delta sync uses `UID SEARCH last_uid+1:*` (new UIDs only, no flag changes)
- No VANISHED handling — expunge detection doesn't exist at all
- No flag-only sync path — delta sync always fetches `BODY.PEEK[]` for every message
- `delta_check_folders()` and `fetch_new_uids()` only look for new UIDs, never for changed or deleted messages

**Gap:** The current delta sync (`imap_delta.rs`) detects new messages and UIDVALIDITY changes but completely ignores flag changes and message deletions. A message marked as read on another client will never sync. A message deleted on the server will persist locally forever. CONDSTORE/QRESYNC addresses both gaps.

### Rust IMAP crate CONDSTORE support

#### async-imap (current — v0.11)

**Supported:**
- `select_condstore()` method — sends `SELECT mailbox (CONDSTORE)`, returns `Mailbox` with `highest_modseq: Option<u64>`. This is a proper first-class API.
- `run_command()` / `run_command_and_check_ok()` / `run_command_untagged()` — raw command execution for anything the typed API doesn't cover.
- `Mailbox.highest_modseq` — parsed from SELECT OK responses by `imap-proto`.

**Not supported (requires raw commands):**
- `UID FETCH ... (CHANGEDSINCE ...)` — no typed modifier; must use `run_command()` and parse the response stream manually.
- `ENABLE QRESYNC` — no typed method; must use `run_command_and_check_ok("ENABLE QRESYNC")`.
- `SELECT mailbox (QRESYNC (...))` — no typed method; must construct the raw command string.
- `VANISHED` response parsing — `imap-proto`'s parser does not handle `VANISHED` responses. They will be silently dropped or cause parse errors.
- `MODSEQ` in FETCH responses — `imap-proto` does not parse the MODSEQ data item from FETCH responses. The per-message mod-sequence is unavailable through the typed API.
- `STORE UNCHANGEDSINCE` — no typed modifier.

**Practical approach with async-imap:** CONDSTORE can be partially implemented:
1. Use `select_condstore()` to get HIGHESTMODSEQ — this works today.
2. Compare cached HIGHESTMODSEQ to determine if flag sync is needed — pure client logic.
3. For CHANGEDSINCE FETCH, use `run_command()` to send `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <modseq>)` and parse the response stream manually.
4. QRESYNC is impractical — VANISHED response parsing would require extending `imap-proto` or building a custom parser for the raw response bytes.

This hybrid approach (typed API for SELECT, raw commands for CHANGEDSINCE) is the same pattern used by Delta Chat when they need IMAP features beyond `async-imap`'s typed API, and is consistent with our existing `raw.rs` fallback pattern.

#### imap-codec / imap-types (duesee — v2.0.0-alpha)

The `imap-types` crate has an `ext_condstore_qresync` feature flag. However, as of March 2026, this feature is explicitly marked **"Unfinished"** in the documentation. The feature flag exists and exposes partial type definitions, but the parser and serializer coverage is incomplete.

Combined with the Gmail SELECT crash (Himalaya issue #641) and the broader maturity issues documented in `docs/imap-ecosystem-assessment.md`, `imap-codec` is not a viable path for CONDSTORE implementation today. If the duesee project matures and completes the `ext_condstore_qresync` feature, it would be the architecturally correct solution — proper type-safe CONDSTORE/QRESYNC with fuzz-tested parsing. But that's a speculative future, not a present option.

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
5. They explicitly chose NOT to implement QRESYNC — "Since Delta Chat is not interested in expunged messages, for better compatibility it is enough to support CONDSTORE extension."

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

Our `move_messages()` in `client.rs` already tries MOVE first and falls back to COPY+DELETE+EXPUNGE. This is correct for CONDSTORE — the mod-sequence increments will be captured by CHANGEDSINCE on next sync regardless of which path was taken.

### IDLE interaction

The RFC does not mandate that IDLE notifications include MODSEQ data. In practice:

- After a CONDSTORE-enabling command, the server MUST include MODSEQ in all untagged FETCH responses, including those generated during IDLE. So if another client changes a flag while we're in IDLE, we'll get a FETCH response with MODSEQ.
- However, EXPUNGE notifications during IDLE do NOT include MODSEQ. With QRESYNC enabled, the server sends VANISHED instead of EXPUNGE, which includes UIDs (but still not MODSEQ).
- EXISTS notifications (new messages) during IDLE never include MODSEQ. The client must issue a FETCH after leaving IDLE to get the new message's metadata.

**Practical impact for Ratatoskr:** Our IDLE handler currently exits IDLE and does a delta check on any notification. With CONDSTORE, the delta check should use CHANGEDSINCE instead of UID SEARCH for flag changes. No architectural change needed — just swap the implementation inside `imap_delta_sync`.

### Data model for mod-seq tracking

The schema is already in place. The `folder_sync_state` table has:

```sql
CREATE TABLE folder_sync_state (
  account_id TEXT NOT NULL,
  folder_path TEXT NOT NULL,
  uidvalidity INTEGER,
  last_uid INTEGER DEFAULT 0,
  modseq INTEGER,            -- ← this column exists but is always NULL
  last_sync_at INTEGER,
  PRIMARY KEY (account_id, folder_path)
);
```

**Changes needed:**

1. **Start writing `modseq`.** `pipeline::upsert_folder_sync_state()` currently hardcodes `modseq = NULL`. Add a `modseq: Option<u64>` parameter and write the server's HIGHESTMODSEQ from SELECT responses.

2. **Rename `_modseq` to `modseq`.** In `pipeline::FolderSyncState`, the field is prefixed with `_` to suppress unused warnings. Remove the prefix and use it in delta sync logic.

3. **Add `capabilities` to connection state.** Need to remember whether the server supports CONDSTORE and/or QRESYNC. This could be a per-session flag set during initial CAPABILITY check, or stored persistently per account. Per-session is simpler and more correct (capabilities can change with server upgrades).

4. **No new tables needed.** Per-message mod-sequences do not need to be stored locally — we only need the mailbox-level HIGHESTMODSEQ for CHANGEDSINCE queries. The server does the heavy lifting of tracking which messages changed.

### Fallback strategy for servers without CONDSTORE

For servers without CONDSTORE (Exchange IMAP via non-Graph path, Courier, hMailServer, miscellaneous corporate servers), the current UID-based approach must remain:

**Current approach (what we have):**
- Delta check: `UID SEARCH last_uid+1:*` to find new messages
- UIDVALIDITY comparison to detect mailbox recreation
- No flag change detection
- No deletion detection

**Improved UID-based fallback (without CONDSTORE):**
1. **New messages:** `UID SEARCH last_uid+1:*` — same as current.
2. **Flag changes:** Periodic `UID FETCH 1:* (FLAGS)` and diff against local DB. For a 50k-message mailbox this returns ~50k small responses (UID + flags only, no bodies). Expensive but unavoidable without CONDSTORE.
3. **Deletions:** `UID SEARCH ALL` returns all existing UIDs. Diff against locally cached UIDs to find deletions. Same cost concern as flag sync.
4. **Frequency:** Flag sync and deletion detection can run at lower frequency (every 5-10 minutes) than new-message detection (every 1-2 minutes or via IDLE).

This is the same approach every IMAP client without CONDSTORE uses. The cost is O(N) per folder where N = message count, vs O(delta) with CONDSTORE.

### Practical gotchas

**Gmail IMAP CONDSTORE quirks:**
- Gmail supports CONDSTORE but NOT QRESYNC. It is reportedly the only major provider in this configuration.
- Thunderbird encountered multiple bugs with Gmail's CONDSTORE: new mail notifications not showing when CONDSTORE is active ([Bug 885220](https://bugzilla.mozilla.org/show_bug.cgi?id=885220)), EXPUNGE responses being lost when CONDSTORE is used without IDLE ([Bug 1124569](https://bugzilla.mozilla.org/show_bug.cgi?id=1124569)).
- Gmail may not consistently report `HIGHESTMODSEQ` in all SELECT responses. Some clients have observed `CONDSTORE` in the capability list but no `HIGHESTMODSEQ` in the SELECT response, which per the RFC means the server doesn't support persistent mod-sequences for that mailbox.
- Since Ratatoskr has a dedicated Gmail API provider, Gmail IMAP CONDSTORE is lower priority — but it matters for users who configure Gmail via generic IMAP rather than the Gmail API path.

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
- Our `ImapFolderStatus.highest_modseq` is already `Option<u64>`, so this check is natural: if `highest_modseq.is_none()`, fall back to UID comparison.

**Mod-sequence resets:**
- A server MAY reset mod-sequences, in which case UIDVALIDITY will also change. Our existing UIDVALIDITY change detection handles this automatically.
- The degenerate case: UIDVALIDITY unchanged but mod-sequences reset. This violates the RFC but could happen with buggy servers. Defense: if HIGHESTMODSEQ < cached value and UIDVALIDITY is unchanged, treat as a reset and do full resync.

### Recommended implementation plan

**Phase 1 — CONDSTORE flag sync (medium effort, high value):**
1. On connect, detect `CONDSTORE` in CAPABILITY response.
2. If supported, use `select_condstore()` instead of `select()`.
3. Store `highest_modseq` from SELECT response into `folder_sync_state.modseq`.
4. In delta sync, compare cached modseq vs server's HIGHESTMODSEQ:
   - If equal: skip flag sync entirely (nothing changed).
   - If server's is higher: issue `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <cached>)` via `run_command()`.
   - Parse the response to extract UID + FLAGS pairs, update local DB.
   - Update cached modseq to server's HIGHESTMODSEQ.
5. Continue using `UID SEARCH last_uid+1:*` for new message detection (orthogonal to CONDSTORE).

This phase uses only `select_condstore()` (typed API) and `run_command()` (raw), both available in async-imap today. No parser extensions needed.

**Phase 2 — Deletion detection (medium effort, medium value):**
1. Without QRESYNC: periodic `UID SEARCH ALL` to get current UID set, diff against local cache.
2. With QRESYNC (if we decide to pursue it): `ENABLE QRESYNC` + SELECT QRESYNC to get VANISHED (EARLIER) in one round trip.
3. QRESYNC requires extending response parsing beyond what async-imap provides — either via raw response parsing or contributing upstream.

**Phase 3 — QRESYNC full sync (high effort, high value for large mailboxes):**
1. Detect QRESYNC in CAPABILITY.
2. Send `ENABLE QRESYNC` via raw command after auth.
3. Send `SELECT mailbox (QRESYNC (<uidvalidity> <modseq> [<known-uids>]))` via raw command.
4. Parse VANISHED (EARLIER) responses and FETCH responses from the SELECT response stream.
5. This requires building a custom response parser or extending `imap-proto`'s parser.

**Recommendation: Start with Phase 1.** It covers the most impactful use case (flag sync, particularly read status) with the least implementation risk. Phase 1 uses existing async-imap APIs and requires no parser work. Phase 2 adds deletion detection. Phase 3 (QRESYNC) should wait until either (a) `imap-codec`'s `ext_condstore_qresync` feature matures, or (b) we hit performance problems that justify building a custom VANISHED parser.

### Sources

- [RFC 7162: IMAP Extensions: CONDSTORE and QRESYNC](https://datatracker.ietf.org/doc/html/rfc7162)
- [RFC 6851: IMAP MOVE Extension](https://www.rfc-editor.org/rfc/rfc6851.html)
- [async-imap Session API (docs.rs)](https://docs.rs/async-imap/latest/async_imap/struct.Session.html)
- [async-imap Mailbox struct (docs.rs)](https://docs.rs/async-imap/latest/async_imap/types/struct.Mailbox.html)
- [imap-codec GitHub (duesee)](https://github.com/duesee/imap-codec)
- [imap-types docs.rs](https://docs.rs/imap-types/latest/imap_types/)
- [Delta Chat CONDSTORE issue #2941](https://github.com/deltachat/deltachat-core-rust/issues/2941)
- [Delta Chat CONDSTORE/QRESYNC issue #200](https://github.com/deltachat/deltachat-core/issues/200)
- [Thunderbird QRESYNC Bug 1747311](https://bugzilla.mozilla.org/show_bug.cgi?id=1747311)
- [Thunderbird CONDSTORE Bug 912216](https://bugzilla.mozilla.org/show_bug.cgi?id=912216)
- [Thunderbird Android CONDSTORE/QRESYNC PR #2607](https://github.com/thunderbird/thunderbird-android/pull/2607)
- [iCloud QRESYNC discussion (Apple Developer Forums)](https://developer.apple.com/forums/thread/694251)
- [Stalwart Mail Server RFCs](https://stalw.art/docs/development/rfcs/)
- [MailCore2 Gmail CONDSTORE issue #297](https://github.com/MailCore/mailcore2/issues/297)
- [Cyrus IMAP RFC Support](https://www.cyrusimap.org/3.10/imap/rfc-support.html)
