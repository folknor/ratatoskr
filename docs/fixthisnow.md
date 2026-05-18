# Fix Now: DB Writer/Reader Boundary Lockdown - Closing PRs

## What this doc is

PRs 0 / 0b / 1 / 2 / 3 / 4 landed the structural split between the read
crate (`db-read`) and the writer crate (`db`), introduced the typed
`WriteConn` / `WriteTxn` / `ReadConn` capability surface, and deleted
the load-bearing escape hatches (`from_arc` constructors, untyped
`with_conn*` on the writer pool, `WriteTxn::as_raw_tx`). The review of
the PR 4 + partial-PR 5 merge identified two follow-up PRs plus a small
housekeeping pass.

Original rationale and the landed-PR step lists live in git history;
start at `729eabe4 db boundary: complete the slice + fix review
blockers` and walk forward.

## PR 4b: close the writer-hidden-behind-reader hole

Three compile-time leaks let writer-shaped functions be publicly typed
as `&ReadDbState` and manufacture `WriteConn::from_raw(conn)` inside
the closure body. Runtime defenses (`SQLITE_OPEN_READ_ONLY` + `PRAGMA
query_only = ON`) reject the actual writes, but every one of these
call sites is swallowing the error via `log::warn!`. JMAP push state,
sync history IDs, JMAP cursors, and Graph webhook persistence are
plausibly **silently failing at runtime since PR 1 opened the read
connection read-only**. Smoke-test these before landing the closures.

1. Delete `ReadDbState::with_conn` and `with_conn_sync` from
   `crates/db-read/src/raw.rs:187-211`. The compile errors are the
   worklist. Affected files (non-exhaustive, ~30 sites):
   `crates/sync/src/state.rs`, `crates/sync/src/pending.rs`,
   `crates/provider-sync/src/keyword_membership.rs`,
   `crates/provider-sync/src/{gmail,jmap,graph}/sync/*.rs`,
   `crates/jmap/src/push.rs`, `crates/graph/src/webhooks.rs`,
   `crates/db/src/db/pending_ops.rs::db_pending_ops_get`.

2. Make `WriteConn::from_raw` `pub(crate)` in `crates/db/src/db/mod.rs`.
   The 13 external callers (1 in `crates/sync/src/state.rs`, the
   rest in `crates/provider-sync/`) get compile errors. Route them
   through `WriterPool::with_write` or a newly-exposed typed entry
   point on `service-state`. The internal callers in
   `crates/db/src/db/queries_extra/message_membership.rs` and
   `label_intent.rs` keep working unchanged.

3. Drop the `WriteTarget` / `WriteTransactionTarget` blanket impls on
   `&rusqlite::Connection` and `&rusqlite::Transaction` at
   `crates/db/src/db/mod.rs:319-401`. Keep the impls on `&WriteConn`
   and `&WriteTxn`. Test fixtures and in-memory schema helpers that
   need raw connections should construct a `WriteConn` via the (now
   `pub(crate)`) `from_raw` instead. The ~85 helper signatures taking
   `&impl WriteTarget` continue to compile without source changes
   because their real callers all hold `&WriteConn` after step 1
   lands.

4. Smoke-test before merging: a long-running session against one real
   JMAP and one Gmail account. Verify
   - `jmap_push_state.consecutive_failures` advances on reconnect.
   - `jmap_push_state.push_state` survives a restart.
   - `jmap_sync_state` rows survive restart.
   - History IDs in the `accounts` row advance under steady-state
     Gmail sync.

   Today the UPDATEs that drive these counters execute against the
   read-only connection. SQLite rejects them; `log::warn!` eats the
   error. If the smoke test shows persistence is currently broken,
   that failure is the primary motivation for this PR.

5. Tighten the lockdown grep: extend
   `db_read_raw_rusqlite_access_is_quarantined` (or add a sibling) to
   also scan `raw.rs` for `pub fn .*&Connection` and `pub fn
   .*&rusqlite::Connection` on `ReadDbState`. Today `raw.rs` is
   exempt from the quarantine, which is precisely where the
   `with_conn(&Connection)` leak lives.

Acceptance: workspace grep for `ReadDbState` paired with
`.with_conn` returns zero hits. `WriteConn::from_raw` is `pub(crate)`
and no external crate names it. The runtime smoke checks pass against
a real account.

## PR 5b: peripheral crate typing + brokkr rules

PR 5 partially migrated `sync`, `provider-sync`, and `calendar`. The
reader-side peripheral crates remain on raw connections and the brokkr
rule set has no new entries.

1. Retype reader-side public APIs to take `&ReadConn`:
   - `crates/smart-folder/src/lib.rs::execute_smart_folder_query`
     takes `&Connection`. The sibling `count_smart_folder_unread`
     already takes `&ReadConn`. Migrate the underlying
     `query_threads` to `query_threads_read` and delete the raw
     variant.
   - `crates/seen/src/ingest.rs::get_self_emails(&Connection, ...)`.
   - `crates/seen/src/backfill.rs` audit (already mostly typed via
     `ReadDbState`).
   - Read paths in `crates/search/` and `crates/calendar/`: audit
     and retype any function whose body is SELECT-only.

2. Retype writer-side helpers in `db` that still take `&Connection`
   instead of `&impl WriteTarget` or `&WriteConn`. The canonical
   mixed-state file is `crates/db/src/db/queries_extra/action_helpers.rs`:
   `thread_exists_sync`, `get_message_ids_for_account_sync`, and
   `delete_threads_for_account_sync` take `&Connection`; the rest
   take `&impl WriteTarget`. Pick one. The same mixed shape appears
   in `accounts_crud.rs`, `contact_carddav.rs`, `draft_lifecycle.rs`,
   `auto_responses.rs`, `compose.rs`, `message_membership.rs`, and
   `extract_reindex.rs`.

3. Flip Cargo dep direction on reader-side crates:
   - `crates/seen/Cargo.toml`: drop `db`, keep `db-read`.
   - `crates/smart-folder/Cargo.toml`: drop `db`, keep `db-read`.
   - `crates/calendar/Cargo.toml`: keep `db` (writer paths exist),
     add `db-read`, ensure read sites use it.

4. Add the missing brokkr `dependency_rule` entries:

   ```toml
   [[dependency_rule]]
   name = "seen-reader-only"
   from = "seen"
   forbid = ["db"]

   [[dependency_rule]]
   name = "smart-folder-reader-only"
   from = "smart-folder"
   forbid = ["db"]
   ```

   Plus rules for any other crate the audit in step 1 reclassifies.

Acceptance: `cargo tree -p seen --depth 1` and `cargo tree -p
smart-folder --depth 1` do not list `db`. No `pub fn` in reader-side
crates takes `&Connection`. New brokkr rules pass `brokkr check`.

## Cosmetic / housekeeping

Pure cleanup; bundle with whichever PR happens to touch the file.

- `crates/db/src/db/mod.rs:115-117`: `apply_standard_pragmas` is a
  one-line alias for `apply_writer_pragmas` with no remaining
  callers. Delete.
- `crates/db/src/db/mod.rs:403-407`: `WriteConn::unchecked_transaction`
  is a one-line forward to `WriteConn::transaction`. Delete unless a
  transitional caller still exists; the design doc never specified
  both names.
- `crates/db-read/src/lib.rs:17-18`: `pub use rusqlite::{Error as
  SqlError, OptionalExtension, Row, params, ToSql}` is fine (none
  are mutating types), but a one-line comment above the re-export
  listing what is allowed and what is banned would save future
  readers a grep through the lockdown crate.
- `crates/service-state/tests/lockdown.rs:81`: the `from_arc` grep
  constructs its banned pattern as `["from", "_arc("].concat()` to
  avoid the test matching its own source. Clever but brittle: a
  future helper named `from_arc_lock` would false-positive. Either
  accept the brittleness or switch to a sentinel comment (e.g.
  `// noqa: from_arc-grep`) the test skips.

## Acceptance criteria (closing roll-up)

PR 4b and PR 5b together close the remaining OPEN markers from the
original acceptance list:

- 11, 23: `WriteConn::from_raw` is `pub(crate)`; no untyped
  `with_conn*` anywhere on the writer- or reader-side state types;
  every closure writing the main DB takes `&WriteConn` and no helper
  accepts `&Connection`.
- 19: `db-read` quarantine extended to cover `raw.rs`'s public API
  surface, not just non-`raw.rs` source files.
- 24: peripheral-crate signatures and matching brokkr
  `dependency_rule` entries.

The original criterion 15 (audit `store`'s app-facing API for the main
DB) stays out of scope. `store` owns its own SQLite databases
(`bodies.db`, the inline image store, the attachment file cache); the
same read/write discipline can be applied separately if and when
those become a concern.
