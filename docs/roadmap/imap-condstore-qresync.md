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
