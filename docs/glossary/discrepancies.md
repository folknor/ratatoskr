# Folders & Labels — Code/Contract Discrepancies

Discrepancies between the code in this repo and the contract in `folders-labels.md`. Findings consolidated from two independent audits (one internal, one external review). Each entry cites the file:line where the divergence lives, the contract clause it violates, and a one-line fix shape.

Severity tiers:
- **P0** — user-visible breakage on the golden path. Fix first.
- **P1** — visible breakage on a less-trodden path, or correct read with broken write.
- **P2** — semantic / hygiene issue that doesn't yet manifest as a user bug but will if untouched.
- **nit** — cosmetic, dead code, or contract-wording problem.

---

## P0 — Inbox click does not apply Inbox semantics

`crates/types/src/sidebar_selection.rs:97`: `SidebarSelection::Inbox` falls into the `None` arm of `folder_id_for_thread_query()`.

`crates/app/src/helpers.rs:97-99`: when `selection` matches the `_ =>` arm, the resulting `label_id` is `None` and `load_threads_scoped(db, scope, None)` is invoked.

`crates/db/src/db/queries_extra/scoped_queries.rs:99-152`: when `label_id` is `None`, `get_threads_scoped` runs the no-label SQL branch (lines 142-152) which selects every thread in scope with no folder filter at all. The `is_all_accounts_inbox(scope, label_id)` check at line 99 only fires when `label_id == Some("INBOX")`; passing `None` skips it entirely.

Effect:
- **Single-account scope**: clicking Inbox returns every thread for that account (drafts, sent, trash, spam, archive — all of it). Contract requires "the strict `INBOX`-label semantics."
- **All-Accounts scope**: clicking Inbox does NOT apply `BROAD_INBOX_EXCLUSIONS` either, because the broad-inbox special-case keys on `Some("INBOX")`. Contract requires the broad inbox view.

Fix shape: either map `SidebarSelection::Inbox` to `Some("INBOX")` in `folder_id_for_thread_query`, or special-case `Inbox` in `load_threads_for_current_view` and call a dedicated `get_inbox_threads_scoped`.

## P0 — Starred and Snoozed routed through `thread_labels` instead of boolean columns

`crates/types/src/sidebar_selection.rs:53-63`: `SystemFolder::Starred` returns `"STARRED"` and `Snoozed` returns `"SNOOZED"`. These flow through `folder_id_for_thread_query()` → `load_threads_scoped` → `get_threads_scoped` (`scoped_queries.rs:126-140`) which joins `thread_labels WHERE label_id = ?`.

But STARRED is filtered out of `thread_labels` by `replace_thread_labels` via `is_message_state_label_id` (`db/src/db/queries_extra/thread_persistence.rs:506-512`, `db/src/db/folder_roles.rs:164-166`). And SNOOZED is purely a Ratatoskr-local flag — no provider ever writes it to `thread_labels`. Both folders' threads live on `threads.is_starred` / `threads.is_snoozed` boolean columns instead.

Effect: clicking Starred or Snoozed in the sidebar returns an empty thread list.

Working query helpers exist (`get_starred_threads`, `get_snoozed_threads` in `scoped_queries.rs:502-510` etc.) but are not called from the main load path.

Fix shape: route `Starred`/`Snoozed` to the dedicated boolean-column query helpers in `load_threads_scoped` before falling through to `get_threads_scoped`.

## P0 — All-Mail folder selection unlikely to match anything

`crates/types/src/sidebar_selection.rs:61`: `SystemFolder::AllMail` → `"all-mail"`. Hits the `thread_labels` join with `label_id = "all-mail"`.

The contract for "All Mail" (single-account only): "shows literally every thread for one account (including drafts, sent, trash, spam)." This requires either no filter (with the single-account scope already restricting) or a dedicated query — not a `thread_labels` join. No provider tags messages with `all-mail` per-message: Gmail's `[Gmail]/All Mail` is a folder pseudo-label, not a per-message label; IMAP's `\All` special-use is a folder, not a flag; JMAP and Graph have no equivalent.

Effect: All-Mail returns empty (or near-empty) instead of every thread.

Fix shape: detect `AllMail` selection in `load_threads_for_current_view` and call `get_threads_scoped` with `label_id = None` (or a dedicated helper) so the no-label branch returns everything in single-account scope.

## P0 — Graph delta sync corrupts thread aggregate labels

`crates/provider-sync/src/graph/sync/persistence.rs:24-65` (`persist_messages`) groups changed messages by thread and calls `store_thread_to_db` per thread with only the messages in this delta page.

`store_thread_to_db` (`persistence.rs:121-163`) calls `set_thread_labels` (`:203-217`), which calls `replace_thread_labels` (`db/queries_extra/thread_persistence.rs:500-530`). `replace_thread_labels` deletes ALL existing `thread_labels` rows for the thread, then re-inserts based on what was passed in.

The "what was passed in" is only the changed messages — not the thread's full message set. If thread T has 5 messages and message 3 changes labels, the next delta cycle replaces T's `thread_labels` based on message 3's labels alone. Folders/labels that exist on the other 4 messages (and which the contract treats as part of the thread aggregate) are silently dropped.

Identical pattern in JMAP: `crates/provider-sync/src/jmap/sync/storage.rs:180-194` (`set_thread_labels` → `replace_thread_labels`) called from per-batch persistence. Identical pattern in Gmail: `crates/provider-sync/src/gmail/sync/storage.rs:126-140` — though Gmail's API typically returns full thread label sets per fetch, mitigating in practice.

Fix shape: either (a) re-fetch all messages for the thread from the local DB before computing the union, or (b) replace `replace_thread_labels` with a merge that adds new labels and removes only labels we know are gone.

## P1 — No provider write-back of replied / forwarded after a reply or forward send

The contract says "Replied and forwarded are derived from outgoing sends, not toggled directly. The action service routes the change to the appropriate provider primitive per the per-provider mapping above."

Read paths are complete on all four providers:
- Gmail: `crates/gmail/src/parse.rs:84-93` (SENT membership + headers / subject)
- Graph: `crates/graph/src/parse.rs:220-237` (`PR_LAST_VERB_EXECUTED` 102/103/104)
- IMAP: `crates/imap/src/client/mod.rs:34-50` (`\Answered`, `$Forwarded`)
- JMAP: `crates/jmap/src/parse.rs:106-107` (`$answered`, `$forwarded`)

Write paths are mostly missing:
- **IMAP**: `crates/imap/src/ops.rs:720-783`. After SMTP send, appends sent-copy to `\Sent` with `(\Seen)` only. Does not `STORE +FLAGS (\Answered)` on the original being replied to, nor `STORE +FLAGS ($Forwarded)` on the original being forwarded. The `_thread_id: Option<&str>` parameter is discarded.
- **JMAP**: `crates/jmap/src/ops.rs:394-468`. Imports the sent message with `$seen`, clears `$draft` on submission success, but does not set `$answered`/`$forwarded` on the original via a follow-up `EmailSet`. The `_thread_id` parameter is discarded.
- **Graph**: `crates/graph/src/ops/send.rs:78-112` (`create_draft_impl`) discards `_thread_id`. Sends are routed through draft + `/send`, not through Graph's `/messages/{id}/reply` / `/replyAll` / `/forward` endpoints (which would set `PR_LAST_VERB_EXECUTED` server-side). No explicit PATCH writes the extended property either.
- **Gmail**: arguably contract-compliant by accident. The contract derives `is_replied` for Gmail from "SENT membership + In-Reply-To/References headers" — so the next sync ingest of the SENT message itself flips the boolean on that new message, the OR-aggregation across the thread lights the glyph, no write-back required. Gap is hidden, not absent.

Effect: IMAP/Graph/JMAP threads where the user replied or forwarded from Ratatoskr never light the ↩ / ↪ glyphs unless another client subsequently sets the flag.

Fix shape: thread the reply/forward intent through the action context to the provider's `send_email`, then issue the appropriate flag/keyword/property write to the original message after SMTP/Graph/JMAP send completes.

## P1 — IMAP custom keywords: incomplete lifecycle

Three separate gaps in IMAP custom keyword handling, all in the read path.

**Initial sync drops custom keywords.** `crates/imap/src/client/sync.rs:308-343` extracts only the known IMAP system flags (`Seen`, `Flagged`, `Answered`, `Forwarded`, `Draft`) before calling `parse_message`. `crates/imap/src/parse.rs:42-60` (`parse_message` signature) takes only those known booleans — it has no parameter for arbitrary `Flag::Custom(_)` keywords. So a freshly-synced mailbox shows zero custom-keyword labels until a CONDSTORE flag-change cycle runs.

**CONDSTORE adds keywords but never removes them.** `crates/provider-sync/src/imap/sync_pipeline.rs:678-720` upserts `kw:{kw}` rows into `labels` and links them via `thread_labels` for any keyword observed in a flag-change. There is no symmetric path that deletes the `thread_labels` row when the server reports a keyword has been removed. Stale `kw:` labels accumulate indefinitely.

**Non-CONDSTORE fallback ignores custom keywords entirely.** `crates/provider-sync/src/imap/imap_delta_janitor.rs:75-106` reads only `(uid, is_read, is_starred)` from the local DB via `get_local_flags_for_folder` and only diffs those two booleans against server flags (line 101). `is_replied`, `is_forwarded`, and any custom keyword changes are invisible to a server that doesn't speak CONDSTORE.

Fix shape: extend `parse_message` to accept a `Vec<String>` of custom keywords; mirror the `kw:` add path with a delete path for keywords gone from the server set; widen the non-CONDSTORE diff to compare full flag sets.

## P1 — Search clauses don't filter by `label_kind`

`crates/smart-folder/src/sql_builder.rs`:

- `build_folder_clause` (line 296-315): joins `labels` for `folder:NAME` matches but does not filter by `label_kind = 'container'`. Will match label rows whose name happens to coincide.
- `build_label_clause` (line 440-454): joins `labels` for `label:NAME` matches but does not filter by `label_kind = 'tag'`. Will match folder rows whose name happens to coincide.
- `build_is_tagged_clause` (line 412-418): `EXISTS (SELECT 1 FROM thread_labels)` — no kind filter. Returns `true` for any thread that's in a folder, even if it has no labels.

`build_in_folder_label_clauses` (line 319-349) is fine — `IN_LABEL_SHORTHANDS` hard-codes specific label_ids per shorthand, no name resolution.

Fix shape: add `AND l.label_kind = 'container'` / `AND l.label_kind = 'tag'` to the joins; add a join in `build_is_tagged_clause` that filters to tag rows.

## P2 — Sidebar "Search here" on a folder uses `label:` prefix

`crates/app/src/ui/sidebar/folders.rs:249` calls `build_search_here_prefix` which always emits `label:NAME` (`crates/app/src/ui/sidebar/search_here.rs:4-19`). A separate `build_search_here_folder_prefix` (lines 22-37) exists and emits `in:NAME` for the universal-folder context, but the user-folder code path doesn't call it.

Visible impact is muted only because `label:` and `folder:` both fail to filter by `label_kind` (see search clause finding above) — so right-clicking a folder still happens to find it. Fix the search clauses and this becomes a real bug.

Fix shape: introduce a third prefix builder for user folders that emits `folder:NAME`, or rename `build_search_here_prefix` and route folders to it.

## P2 — Master-category sync cadence leaves a window for orphan `cat:` rows

`crates/graph/src/label_sync.rs:graph_label_sync` correctly fetches `/me/outlook/masterCategories` and upserts `cat:{displayName}` rows with `label_kind = 'tag'`. Called on initial sync (`provider-sync/src/graph/sync/mod.rs:183`) and every 20th delta cycle (`:368`).

Two residual gaps:

1. A category created in Exchange after the most recent `graph_label_sync` run will be referenced in `thread_labels` (via `parse_graph_message` → `get_folder_and_label_ids_for_message` → `format!("cat:{cat}")`) before any `labels` row exists for it. Up to ~20 delta cycles of latency before the LABELS sidebar section enumerates it and before `add_label`/`remove_label` action gates accept it.

2. Per-message ad-hoc categories that aren't in the master list (Exchange does support these) are never reachable via `graph_label_sync` and become permanent orphans in `thread_labels`.

Fix shape: upsert a `cat:{name}` row into `labels` at the same point where `provider-sync/src/graph/sync/persistence.rs:152` writes to `thread_labels` for an unseen category — belt-and-braces with `graph_label_sync` rather than relying on it alone.

## P2 — Action-pipeline `add_label` blocks unknown labels

`crates/service/src/actions/label.rs:32-42` reads `label_kind` via `get_label_kind_sync` (`db/queries_extra/action_helpers.rs:24-39`), which queries the `labels` table. Returns `ActionError::not_found("label not found for this account")` when the row isn't present.

This is correct in spirit — we shouldn't add a label that doesn't exist — but cascades from any of: the cat-orphan window above; an IMAP custom keyword the user hasn't seen on any incoming message yet; a JMAP custom keyword in the same situation.

Fix shape: if root causes (P1 / P2 above) are fixed, this becomes self-resolving. Otherwise consider auto-creating a `kw:`/`cat:` row on the fly when the label_id matches a known prefix.

## nit — Contract wording for JMAP keyword identity is ambiguous

`folders-labels.md` "Identity" section: "JMAP keywords - keyword string with the JMAP keyword prefix conventions."

Code uses `kw:{keyword}` (`crates/jmap/src/ops.rs:295`, `crates/provider-sync/src/jmap/sync/storage.rs:301`), reusing the IMAP convention. The contract sentence reads either way — "use the JMAP convention" (which is no prefix) or "with our prefix convention as applied to JMAP keywords."

Fix shape: tighten the contract to either `jmap-{keyword}` or `kw:{keyword}` (probably the latter, matching what the code does and aligning JMAP with IMAP). Code change only if the contract says otherwise.

## nit — `is_user_visible_keyword` is stricter than RFC 5788

`crates/db/src/db/folder_roles.rs:182-184`: returns `!keyword.starts_with('$') && !is_reserved_imap_system_keyword(keyword)`. The `$` prefix is reserved by IETF for IMAP system keywords (RFC 5788 §2.1), so excluding all `$`-prefixed keywords is safer than excluding only the named reserved set — but contract section "Per-Provider Mapping → IMAP" implies the named-set semantics. Probably intentional. Worth either documenting in code or relaxing the contract wording.

## nit — `is_draft` parsed but unused

`crates/imap/src/client/mod.rs:312, 327, 382, 395` reads `Flag::Draft` and threads `is_draft: bool` through `parse_message` and onto `ParsedImapMessage` (`crates/imap/src/types.rs:72`). Nothing in `crates/provider-sync/src/imap/` reads it. Contract-compliant (drafts are folder-membership-derived) but dead plumbing that suggests an unfinished intent.

Fix shape: delete it, or wire it to fold cross-folder `\Draft` markers into Drafts membership.

## nit — `STARRED` canonical id has no `labels` row anywhere

`crates/db/src/db/folder_roles.rs:91-98` declares `STARRED` in `SYSTEM_FOLDER_ROLES` with no provider mapping. `crates/provider-sync/src/gmail/sync/labels.rs:33` filters Gmail's STARRED system label out of the labels-table sync via `is_message_state_label_id`. `crates/db/src/db/queries_extra/thread_persistence.rs:506-512` filters STARRED out of `thread_labels` writes. `replace_thread_labels` won't write it; no provider sync inserts a `labels` row for it.

So the canonical `STARRED` label_id is purely virtual: it's used for navigation, decorated as a "folder" in the sidebar, and routed through the `is_starred` boolean column at query time. Contract-compliant, but non-obvious to a reader who assumes `SYSTEM_FOLDER_ROLES` entries correspond to `labels` rows.

Fix shape: add a doc-comment near the `STARRED` entry in `folder_roles.rs` explaining that this id is virtual and exists only as a navigation handle.

## nit — Importance label rows depend on folder sync running before message ingest

`crates/provider-sync/src/graph/sync/folders.rs:84` upserts `importance:high` / `importance:low` rows during `sync_folders`. Messages parse-time path (`crates/graph/src/parse.rs:94-96`) appends `"importance:high"` / `"importance:low"` to `label_ids` whenever Graph reports `importance` other than `normal`. If `sync_folders` hasn't run yet (or has failed), a thread can reference `importance:high` in `thread_labels` before the labels row exists.

Practically harmless — the upsert is idempotent and `add_label` is user-triggered — but the dependency is invisible. Same shape as the Graph cat-orphan window.

Fix shape: same as cat fix — upsert importance rows belt-and-braces at message-persist time, or document the dependency.

---

## Summary table

| ID | Severity | Area | Status |
|---|---|---|---|
| Inbox click → unfiltered | P0 | Sidebar routing | Open |
| Starred/Snoozed via `thread_labels` | P0 | Sidebar routing | Open |
| All-Mail via `thread_labels` | P0 | Sidebar routing | Open |
| Delta sync replaces thread labels from partial set | P0 | Graph/JMAP/Gmail sync | Open |
| Send doesn't write replied/forwarded | P1 | IMAP/Graph/JMAP send | Open |
| IMAP custom keyword lifecycle | P1 | IMAP sync | Open |
| Search clauses no `label_kind` filter | P1 | Smart-folder SQL | Open |
| Sidebar "Search here" uses `label:` for folders | P2 | Sidebar UX | Open |
| `cat:` orphan window | P2 | Graph sync cadence | Open |
| `add_label` blocks unknown labels | P2 | Action pipeline | Open (cascades from P2s above) |
| JMAP keyword identity contract ambiguous | nit | Contract wording | Open |
| `is_user_visible_keyword` stricter than RFC 5788 | nit | Code/contract alignment | Open |
| IMAP `is_draft` parsed but unused | nit | Dead plumbing | Open |
| `STARRED` virtual id is non-obvious | nit | Documentation | Open |
| Importance row sync depends on folder sync | nit | Sync ordering | Open |
