# Labels Unification: Post-Landing Discrepancies

Review of commit `0eac3f9c` ("labels-unification: split storage and replace auto-collapse with groups") against `docs/labels-unification/redesign.md` and `docs/glossary/folders-labels.md`.

Findings are appended as each slice review completes. Each item names file:line, what's wrong, and the spec line or invariant being violated.

---

## Slice 1 - schema + types foundation

RESOLVED in this branch. Type split landed (`SystemFolder` real-row-only +
`VirtualView { Starred, Snoozed, AllMail }`), STARRED removed from
`SYSTEM_FOLDER_ROLES`, `folders_account` index added, `label_groups.id`
gains AUTOINCREMENT, `LabelId` doc-comment narrowed to tag-only,
`get_labels` exclusion-list rationale captured in code comment.
`insert_folders_batch` deferred to slice 3 (write helpers).
`supports_keywords` gate deferred to slice 4 (action pipeline).

---

## Slice 2 - DB query layer split

RESOLVED in this branch. `upsert_folder_from_mutation_sync` no longer
derives `is_undeletable` (user creates never carry the system flag; on
conflict the existing flag is preserved) and the precondition that
`parent_id` already exists is documented at the helper. The three dead
`db_upsert_label_coalesce` / `db_delete_labels_for_account` /
`db_update_label_sort_order` helpers (and their associated
`LabelSortOrderItem`) were deleted along with the dead
`query_visible_labels` / `CrossAccountLabel` / `LabelBacking` shapes -
no callers remained post-split. `upsert_labels` now ORs `is_undeletable`
on conflict so the `importance:*` invariant survives sync clobber.
`ThreadLabel.label_kind` removed (every entry is a group);
`ResolvedLabel.label_kind` removed in app crate. `system_label_ids`
renamed to `system_folder_ids`. `query_thread_label_decorations` now
explicitly GROUPs by `(thread_id, group_id)` so a future variance in
name/color cannot duplicate pills. `get_user_folders_for_account_sync`
documents why it filters on `is_undeletable = 0`.
`filtered_thread_labels` renamed to `filtered_membership_ids` (used by
both folder and label thread-junction writers).

`TestDbLabelRow.label_kind` / `label_type` wire fields stay until
slice 6 deletes the harness compatibility shim.

---

## Slice 3 - provider sync split

RESOLVED in this branch. `recompute_thread_keyword_labels` drops the
LIKE filter (IMAP-account threads only carry `kw:*` rows by provider
definition, so the destructive replace is safe). Gmail
`set_thread_labels` pre-creates placeholder `labels` rows for any
user-label IDs referenced by the messages so the FK in
`replace_thread_labels` cannot fire ahead of the next `sync_labels`
cycle. IMAP `is_undeletable` narrows to `folder.special_use.is_some()`,
so a user-named "Drafts" on a server without SPECIAL-USE is no longer
trapped as system. Gmail labels module renamed
(`persist_folders_and_labels`) with a docstring naming the partition,
Graph folders helper renamed (`persist_folders_and_importance`), and
`"commit labels:"` error strings on folder inserts read
`"commit folders:"` across IMAP / JMAP / Graph. `is_graph_tag_id`
lifted to `common::folder_roles`. The stale
`test_get_folder_ids_for_draft` was updated to use the canonical
"DRAFT" id that production callers actually produce.

---

## Historical (pre-landing audit, retained for context)

The previous version of this file audited the pre-refactor auto-collapse design. Items resolved by the landing commit:

- **Section 4 empty in All-Accounts scope** - superseded; sidebar LABELS now renders `label_groups` rows.
- **Cross-account grouping missing trim normalization** - superseded; identity is now `label_groups.id`.
- **Per-account vs cross-account unread counts** - superseded by `get_label_group_unread_counts`.
- **Cross-account label creation / deletion UI without action** - partly superseded; re-verify wiring in slice 5.
- **`label_color_overrides` write path missing** - superseded; table dropped, per-row `user_color_bg/fg` columns replace it. Re-verify wiring in slice 5.
- **`is_folder_based_provider` dead-code trio** - re-verify removal in slice 2/5.
- **Spec text drift in `problem-statement.md`** - moot, file is superseded by `redesign.md`.

Carryover (not directly addressed by the landing commit):

- **IMAP `supports_keywords` action-service gate** - see slice 1 MEDIUM.
- **Label dispatch thread-level vs per-message** - re-evaluate in slice 4.

---

## Slice 4 - action pipeline + label_group composite

RESOLVED in this branch.

- Composite retry preflight is reachable: `apply_label_group_with_kind` /
  `remove_label_group_with_kind` take a `DispatchKind` (Initial vs Retry).
  The pending-ops drainer dispatches via `_retry` variants. On a Retry,
  the local helper consults `thread_label_groups(T, G)` and returns
  `LocalStep::Skip` if user intent has reversed since enqueue; the
  composite resolves as `Success` without dispatching member writes.
- Per-member dispatches inside `dispatch_member_ops` run against a
  clone of the context with `suppress_pending_enqueue = true`. The
  underlying `add_label_with_provider` / `remove_label_with_provider`
  still go through their normal enqueue helper, which is a no-op under
  the suppress flag. Composite caller enqueues a single
  `applyLabelGroup` / `removeLabelGroup` row covering the failed
  members via `enqueue_composite_if_local_only`. The drainer never sees
  raw `addLabel` retries from a composite.
- `dispatch_member_ops` continues past per-member `Failed` outcomes so
  a single hard error does not abandon the rest. LocalOnly takes
  precedence over Failed in the aggregate so the composite-retry path
  activates whenever any member is retryable.
- `ensure_prefixed_tag_label` uses `INSERT ... ON CONFLICT ... DO
  UPDATE SET is_undeletable = OR-merge`, repairing a stale
  `importance:*` row whose flag was cleared by a pre-invariant sync.
- `set_keyword_batched` returns `ProviderError::Client` for a
  non-`kw:` label id rather than silently succeeding. With the
  composite calling per-member, that surfaces a real provider failure
  instead of marking the member Success.
- `MailActionIntent::AddLabel` / `RemoveLabel` doc-comment names the
  Settings/undo-only contract so a future contributor cannot wire a
  context-menu item to the per-account variant by accident.
- Tests added: local-step initial insert/delete, unknown-group error,
  zero-member apply, public composite under no-provider path asserts a
  composite-typed retry row is enqueued and no per-member raw
  addLabel/removeLabel rows leak into `pending_operations`.

Deferred:
- `supports_keywords` action-service gate (still missing; needs the
  resolver-level filter that requires UI work alongside).
- Strict atomicity of TLG-insert + per-member `thread_labels` inserts
  (current shape is N+1 transactions; the journal + composite-retry
  contract above tolerates partial-failure mid-fanout).
- Undo of Apply uses the CURRENT member set rather than the historical
  one. Documented as a known trade-off in `architecture.md` "Composite
  operations".

---

## Slice 5 - UI + command palette

RESOLVED in this branch (partial; see "Deferred" below).

- Reading-pane CRITICAL is closed structurally by slice 2's removal of
  `label_kind` from `ResolvedLabel` / `ThreadLabel`. There is no
  compatibility-discriminator field left to mislead a future raw-label
  writer.
- Remove-Label picker scope: `get_thread_label_groups_sync` returns
  every group the thread renders. The composite already removes from
  BOTH paths symmetrically (the local helper reads
  `thread_labels` rows whose member labels belong to the group and
  dispatches per-member RemoveLabel, then deletes any local TLG row).
  No picker change needed; the original audit assumed
  `RemoveLabelGroup` only touched TLG, which is not what the composite
  does.
- `handlers/labels.rs` write stubs retyped to `(account_id, label_id)`
  + display-name variants; the pre-split `normalized_name` parameter
  is gone, and log lines name both axes.
- Settings "Add Label" copy rewritten to reflect per-account scope.
- `handle_label_op` log line renamed from "cross-account labels" to
  "per-account labels".
- `build_command_args` for `EmailAddLabel` / `EmailRemoveLabel` /
  `NavigateToLabel` logs a warning when `item.id` is non-numeric (the
  only producer always emits `i64::to_string()`, so a non-numeric id
  is a programmer error worth surfacing).
- Dead `(NavigateToLabel, CommandArgs::NavigateToFolder)` dispatch arm
  removed; `CommandArgs::NavigateToFolder` variant deleted - nothing
  in the palette pipeline produced it post-split.
- Sidebar labels: tuple binding renamed `(folder, group_id)`,
  `has_account_labels` variable renamed `has_label_groups`.
- `MailActionIntent::AddLabel` / `RemoveLabel` doc-comment names the
  Settings/undo-only contract (slice 4 addition reaffirms this).

Deferred (real product/UI work):
- Settings "Label Groups" view (create-group, member picker,
  member-incompatibility enforcement for `importance:*` pairs).
  Requires new types in `ui/settings/types`, new editor sheet, new
  message variants, and Service IPC for group CRUD. Tracked as the
  primary outstanding piece of the labels-unification rollout.
- Per-account "Add Label" account picker dropdown - depends on the
  same Service IPC surface as group CRUD.
- `label.recolor` / `label.rename` / `label.delete` action-service
  handlers + the corresponding `recolor_label_async` save-path wiring.
  Current stubs preserve the call-site shape; `LabelEditorState`
  carries the editor data but no Service IPC ferries it across yet.
- Add-Label picker annotation showing which groups have members on
  the active account (defensible UX gap, not a correctness bug; the
  composite handles the no-member case as spec-correct local intent).

---

## Slice 6 - smart folder + dev-seed + docs + harness

### CRITICAL

**Sync-harness Lua fixtures assert on dropped columns and the wrong table.** `crates/app/tests/sync-harness/graph-master-category-label-sync.lua:67-71, 76-79, 85-86` asserts `work.label_kind == "tag"`, `work.label_type == "user"`, `work.color_bg == "#e74c3c"`, `work.color_fg == "#ffffff"`. `crates/app/tests/sync-harness/jmap-mailbox-secondary-create-import.lua:117` asserts `label.label_kind == "container"` on a row representing a JMAP mailbox (a folder). Per `redesign.md:96-98`: `label_kind` and `label_type` are dropped post-split (table-of-origin is structural), and `color_bg`/`color_fg` are renamed to `server_color_bg`/`server_color_fg`. The reason the scripts have not exploded yet is the compatibility shim in `crates/service/src/handlers/test_helpers.rs:1274-1336` (`read_harness_labels`) that synthesises the old shape from a `folders UNION labels` query, manufacturing `label_kind` (`'container'` for `folders` rows, `'tag'` for `labels` rows) and aliasing `COALESCE(user_color_bg, server_color_bg) AS color_bg`. So the harness silently keeps passing while asserting on names that are no longer part of the schema, and JMAP's mailbox-create test verifies a row in `labels` that actually lives in `folders` - the very mix the split was meant to make impossible. Slice 6's `+1 -0` "realignment" only adds a `storage_splits_folders_labels_and_groups` `@covers:` tag; it does not realign the assertions. Either: delete `read_harness_labels`'s synthesised columns and rewrite the Lua to query `folders` and `labels` separately (`state.folders` / `state.labels`), or keep the shim but document loudly that the harness is testing a fiction, and add explicit coverage that the underlying split landed. Coverage tag without aligned assertions is worse than no tag.

**Dev-seed never exercises any group-rendering path.** `crates/dev-seed/src/*` contains zero references to `label_groups`, `label_group_members`, `thread_label_groups`, or `importance:high`/`importance:low`. Per `redesign.md:381-383` ("Dev-seed may ship pre-populated groups for the demo state") and the redesign:215 invariant ("The sidebar's LABELS section starts empty on a fresh install"), zero groups is technically valid - but per `redesign.md:332` ("Lua service-harness scripts should exercise the count path at fixture scale, with at least one cohort that stresses the UNION across thousands of threads with many groups") the union-heavy hot path needs real exercise somewhere, and dev-seed is the only fixture surface that runs end-to-end. The user-visible consequence: the entire LABELS-section sidebar rendering, the `get_label_group_unread_counts` UNION code path, the `label:` smart-folder operator, the `is:tagged` operator, the message-pill decoration through the group union, and the Graph `importance:*` synth all go through dev-seed without being exercised. `cargo run -p app --features dev-seed` shows the developer an empty LABELS section by design, but it also shows them zero coverage of the largest new code path in the refactor. Add a seed step that creates 2-3 `label_groups`, attaches at least one `thread_label_groups` row (local intent path) and one `thread_labels`-via-member row (provider-observable path) per group, and on Graph accounts inserts the two `importance:*` `labels` rows per `redesign.md:378`.

### HIGH

**Smart-folder `is:tagged` SQL bypasses the canonical aggregate function.** `crates/smart-folder/src/sql_builder.rs:413-423`. `redesign.md:329` mandates: "one query function (`get_label_group_unread_counts` in `crates/db/src/db/queries_extra/scoped_queries.rs`) is the single source of truth for sidebar counts. All consumers route through it. No ad-hoc UNION at call sites." The `is:tagged` builder writes an inline `(EXISTS thread_label_groups) OR (EXISTS thread_labels JOIN label_group_members)` UNION right in the smart-folder SQL. Same shape for the `label:` builder (sql_builder.rs:445-472). Spec line 329 covers sidebar counts specifically, but the principle ("the UNION is one thing, defined once") applies - drift is exactly the bug shape `docs/glossary/discrepancies.md` is named for. Today the smart-folder UNION and the sidebar UNION are textually similar by accident, not by construction; the moment a future refactor adds `WHERE tlg.deleted_at IS NULL` to one and forgets the other, the smart-folder count and the sidebar count diverge. Either extract the rendering-paths union into a parameterised SQL fragment helper that both call sites use, or accept the duplication explicitly with paired comments naming the spec line. Today neither is true.

**`docs/search/app-integration-spec.md:946-950` "Account scoping for label/folder typeahead" contradicts the new model.** The text reads: "When the query already contains an `account:` operator, `label:` and `folder:` typeahead results are scoped to that account... If the same label name exists on multiple accounts, append the account name in the `detail` field for disambiguation: 'Clients (Work Account)'." Post-split, `label:` binds to `label_groups.name` (no `account_id` column on `label_groups`). The 3.4 row at line 939 correctly drops the account filter from the SQL, but the prose section directly below (3.4.x) still describes per-account scoping and per-account disambiguation that no longer makes sense for groups. Worse, line 1104 still emits `account:Acct label:Name` as the format string for "Account-specific label" sidebar items - but a label *group* has no account, so `account:` + `label:` is a meaningful intersection only if the user understands that "Account-specific label" really means "any group with a member on that account." Rewrite section 3.4.x to make explicit that `label:` is group-scoped and cross-account; either drop the `account:` prefix from the sidebar-to-query fan-out for label rows, or document that the combination intersects "group named X" with "thread on account Y" rather than scoping the label list.

**`docs/search/implementation-spec.md` is missing a `label:` operator section.** The file documents `folder:`, `in:`, `is:tagged`, `has:contact`, `type:`, and `account:` SQL shapes (lines 173-209) but skips `label:` entirely. This is the operator the refactor changed most substantially: the SQL went from a `thread_labels JOIN labels WHERE label_kind='tag' AND LOWER(name)=LOWER(?)` join to the new double-EXISTS-via-`label_groups`-with-UNION-on-rendering-paths shape (sql_builder.rs:445-472). The is:tagged section at line 198-199 names the new shape ("explicit label group, either through `thread_label_groups` or through `thread_labels` joined to `label_group_members`") - `label:` deserves at least as much. Without it, a contributor reading implementation-spec to understand the search SQL has no documentation for the most-different operator.

**Architecture doc "Adding a New Email Action" never mentions the `ApplyLabelGroup` / `RemoveLabelGroup` composite.** `docs/architecture.md:237-254`. The checklist (variant in `MailActionIntent` → variant in `MailOperation` → wire mirror → resolve_intent → completion_behavior → service-side function → batch.rs → undo) is unchanged from the pre-split version. Slice 4 lands the most architecturally novel new action in the codebase: a *composite* operation (`redesign.md:255` "Each is a single composite operation at the action-planner level. Internal fan-out (per-member provider dispatches) happens inside the service-side action function, hidden from the planner."). Composites differ from any other action because the per-member dispatches enqueue per-member retries (slice 4 CRITICAL on retry preflight). The checklist is binding ("Enforcement: `MailOperation` is an exhaustive enum, mirrored 1:1") but says nothing about how a composite picks its preflight, its retry shape, its fan-out, or how `MailUndoPayload` pairs an `ApplyLabelGroup` with a `RemoveLabelGroup`. Add a "Composite operations" callout citing `actions/label_group.rs` as the worked example, and pin the retry-preflight contract to the slice-4-discovered failure shape so the next composite contributor cannot reproduce that bug.

### MEDIUM

**Dev-seed `Account.labels: Vec<(String, String)>` is now a mixed bag of folder IDs and label IDs under one collection.** `crates/dev-seed/src/accounts.rs:273-274` ("Map from folder or tag label name to provider id"), populated at 318 (folder IDs from `SYSTEM_LABELS` inserts) and 337 (label IDs from user-label inserts). Consumers in `chats.rs:500` and `threads.rs:530, 549` do `acc.labels.iter().find(|(name, _)| name == "INBOX")` to get folder IDs and `acc.labels.iter().find(|(name, _)| name == "Clients")` to get label IDs from the same vec, then insert one into `thread_folders` and the other into `thread_labels`. The fact that the right one lands in the right table is by convention - INBOX/SENT/TRASH/etc. are folders, "Clients"/"Projects" are labels - but the type doesn't enforce it. Per the redesign's "make the right thing the only thing" principle (`redesign.md:142`), `Account` should carry `folders: Vec<(String, FolderId)>` and `labels: Vec<(String, LabelId)>` as two distinct collections. Today's mix would silently FK-violate the moment a future preset names a label "INBOX" or a folder "Clients."

**Dev-seed chats and threads use a local binding named `label_id` to hold a folder ID.** `crates/dev-seed/src/chats.rs:500-506` and `crates/dev-seed/src/threads.rs:530-536` both write `if let Some((_, label_id)) = acc.labels.iter().find(...)` and then `INSERT INTO thread_folders (..., folder_id) VALUES (..., ?3)` passing `label_id`. Per `docs/glossary/folders-labels.md:32-42` ("If you're reading code and the name says 'label' but the value is a folder, treat that as a bug to fix, not as a permission to mix terms in new code") and the code-identifier rule there, the binding must be renamed `folder_id`. Symptomatic of the MEDIUM above.

**Smart-folder test fixture writes a `label_groups` row with a hardcoded integer PK and no test for the renamed-group lookup.** `crates/smart-folder/src/sql_builder.rs:622-625` (test seed) inserts `label_groups (id=1, name='Projects', ...)`. The `label_filters_by_label_group_name` test (line 755-762) confirms `label:Projects` matches threads via the union path. But there is no test for the rename-resolution behaviour stated in `redesign.md:313` ("Persisted smart folders store the textual query, so a group rename changes which group a name-based query resolves to") and `redesign.md:367` ("Smart-folder references to G continue to work because they bind to `group_id`, not name") - which contradict each other. The landed code binds by name, so 367 is the wrong half - but the test suite would benefit from pinning the actual behaviour: rename the group to "Renamed", reparse `label:Projects`, confirm zero hits; reparse `label:Renamed`, confirm hits. Closes the spec contradiction by test-as-source-of-truth.

**Dev-seed `pinned_searches.rs` snapshot query is correct but the function name lies.** `crates/dev-seed/src/pinned_searches.rs:128-153` `load_account_inbox_snapshot` now joins `thread_folders` for `folder_id = 'INBOX'`. The function name is fine, but the previous version's call sites and downstream variable names (not shown in the diff but inherited) likely still treat the return as a "label" snapshot. Spot-check downstream usage; if any consumer renames `label_id` to `folder_id`-bearing variables, fix here.

**`docs/glossary/discrepancies.md:112` still references `docs/labels-unification/problem-statement.md` as the source of the auto-collapse design.** The text says "superseded by `docs/labels-unification/redesign.md`" so the reference is at least labeled stale, but `problem-statement.md` still exists on disk (`docs/labels-unification/problem-statement.md`). Per `redesign.md:15` ("supersedes `docs/labels-unification/problem-statement.md` in full"), the right action is either delete the file or add a top-of-file banner pointing every reader to `redesign.md`. Today a new contributor opening the file sees a 19.8 KB design doc with no in-file indication that the entire document is wrong.

**`docs/glossary/folders-labels.md:55` Gmail row drops `STARRED` from the system-folder list silently.** The pre-split row listed `INBOX, SENT, DRAFT, TRASH, SPAM, STARRED, IMPORTANT, CHAT, CATEGORY_*` as folders. Post-split, `STARRED` is virtual (`folders-labels.md:153` "Virtual navigation IDs are not folder rows: `STARRED` maps to `threads.is_starred`"). The new row drops `STARRED` from the list - correct per redesign:104 - but does not annotate the change at the row, leaving a reader to spot the absence by careful comparison with the old text. Add a footnote or a short prose sentence ("STARRED maps to the `is_starred` thread boolean, not a `folders` row; see Virtual navigation IDs below").

### LOW

**`crates/smart-folder/src/sql_builder.rs:691-706` test comment says "tag-kind label" but the new vocabulary is "tag label".** "Receipts is a folder on acc1. Projects is a tag label and label group, so it must not match `folder:`." Fine post-split, but the per-line term shift from "tag-kind label" (legacy) to "tag label and label group" (new) is awkward - "label" alone, with the rest left to context, is consistent with `folders-labels.md`. Stylistic.

**`docs/architecture.md:288` "Per-message membership store" patch still describes thread aggregates being written to `thread_labels`.** The text says: "The thread-level `kw:%` rows in `thread_labels` are not written directly; they are recomputed from the union of `message_keywords` rows for the thread's messages by `recompute_thread_keyword_labels`." Post-split this is correct - `thread_labels` is labels-only - but the slice-3 CRITICAL flagged that `recompute_thread_keyword_labels` still scopes its delete with `WHERE label_id LIKE 'kw:%'`. The architecture-doc paragraph promises that the per-message store pattern handles removal, but the implementation still scopes the rollup destructively to `kw:` prefix only. Either the architecture-doc claim or the implementation needs to be aligned (the implementation, per slice 3).

**Smart-folder test `is_starred_uses_thread_aggregate_with_message_date_filters` validates the new fix-shape but not the original bug symptom.** sql_builder.rs:728-738 asserts that `is:starred` + `after:2500` returns t3. The original `docs/glossary/discrepancies.md` motivating example was "Starred This Week shows 24 unread threads when opened and a 0 pill" - a pill-vs-list mismatch. The new test confirms list behaviour but doesn't pair it with a `count_matching` assertion using the same parsed query, which is what would actually pin the fix. `count_matching_forced_unread_uses_thread_aggregate` (line 826-836) exists but tests a slightly different combination (`is_unread = Some(true)` set independently). Add a paired test that runs `query_threads` and `count_matching` over identical parsed input and asserts `threads.len() as i64 == count`.

**`docs/glossary/folders-labels.md:259` "message_keywords table" doc text reads "The thread-level `kw:%` rows in `thread_labels` are derived from the union of `message_keywords` rows".** True post-split, but the LIKE-prefix scoping issue from slice 3 (CRITICAL) makes "derived from" slightly misleading - currently the rollup deletes only `kw:%` and inserts `kw:%`, leaving non-`kw:` rows untouched. The text should match the resolved behaviour after slice 3 is fixed, or note today's partial-scoping behaviour.

**Dev-seed `Account.labels` doc comment "Map from folder or tag label name to provider id" reads as resigned to the mix.** `crates/dev-seed/src/accounts.rs:273`. The comment acknowledges the mix without flagging it as wrong. Stylistic; pair with the MEDIUM above when restructuring.

---

**Verdict.** The smart-folder SQL implements `label:` and `is:tagged` against the union of `thread_label_groups` and `thread_labels`-via-members, the docs prose generally reflects the new shape, and the search docs have been updated for the new SQL - the bones of slice 6 are in place. The two CRITICALs are independent of each other and both worth blocking on: the harness "realignment" is a single `@covers:` tag without any change to the assertions that still hit dropped columns through a compatibility shim, so the coverage claim is hollow; and dev-seed seeds zero groups, leaving the entire UNION rendering path, `importance:*` synth, `is:tagged`, `label:`, and the new sidebar LABELS section unexercised by the only end-to-end fixture in the repo. The HIGH on ad-hoc UNION in smart-folder is the discrepancy-class bug for next time. Architecture and search docs need their composite-action and `label:` operator gaps closed.
