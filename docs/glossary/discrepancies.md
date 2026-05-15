# Glossary Discrepancies

Audit date: 2026-05-15

Findings consolidated from five independent audits of the codebase against `docs/glossary/folders-labels.md`:

- Agent A: provider sync (`crates/provider-sync/`, `crates/gmail/`, `crates/graph/`, `crates/imap/`, `crates/jmap/`).
- Agent B: UI excluding sidebar (`crates/app/src/ui/` minus `sidebar/`).
- Agent C: data model (`crates/core/src/db/`, `crates/db/`, `crates/common/`, `crates/types/`).
- Agent D: action pipeline (`crates/core/src/actions/`, `crates/service/`, `crates/service-api/`, `crates/service-state/`, `crates/common/src/ops.rs`).
- Outside review (separate engagement, full-workspace scope).

The findings are grouped by theme rather than by source, since most issues cross crate boundaries. The outside review largely corroborated the agent findings (good cross-validation) and added two new themes plus several deeper pointers; both are integrated below.

**Editing rule:** theme numbers are stable identifiers. New themes get the next free number; resolved or merged themes keep their number with a `(resolved)` / `(merged into Theme N)` note. Do not renumber existing themes - downstream issues, commits, and conversations may reference them by number.

---

## Theme 1: Message state never extracted

The glossary mandates `is_replied` / `is_forwarded` per-message booleans with per-provider extraction. None of this exists.

- **Schema gap.** `crates/db/src/db/schema/02_mail.sql:86-135` has no `is_replied` or `is_forwarded` columns on `messages`. `DbMessage` (`crates/db/src/db/types.rs:88-117`) mirrors the gap; the row mapper at `crates/core/src/db/queries.rs:22-54` cannot surface them.
- **Shared parsed-message shape lacks the fields.** `crates/common/src/parsed_message.rs:9` defines the common parsed shape with no `is_replied` / `is_forwarded` slot, so even if a provider parser extracted them there is nowhere to put them.
- **Persistence and read paths don't carry them either.** `crates/db/src/db/queries_extra/message_persistence.rs:3` doesn't insert them; `crates/db/src/db/queries_extra/thread_detail.rs:17` doesn't select or expose them.
- **Extraction missing in every provider.** IMAP `parse_message` (`crates/imap/src/parse.rs:47-57`) reads only `is_read`/`is_starred`; never `\Answered` or `$Forwarded`. IMAP flag handling in `crates/imap/src/types.rs:164` has no replied/forwarded fields. JMAP `parse.rs:103-105` reads `$seen`/`$flagged` only; ignores `$answered`/`$forwarded`. Graph `parse.rs:90-108` never reads `PR_LAST_VERB_EXECUTED` (MAPI 0x1081), and the Graph request shape at `crates/graph/src/types.rs:216` does not request the extended property. Gmail `parse.rs` never derives reply/forward state from `SENT` thread membership + `In-Reply-To`/`References` or `Subject: Fwd:` prefix.
- **No UI surface.** `thread_card` (`crates/app/src/ui/widgets/cards.rs:23-27, 91-93`) has no reply / forward glyph slots; the thread list and reading pane (`crates/app/src/ui/reading_pane.rs:41-46, 696, 836-838`) render no `↩` / `↪` indicators anywhere. The glossary requires these inline.

**Severity: High.** Whole-feature gap, but the work decomposes cleanly: schema migration -> per-provider extraction -> column rendering.

---

## Theme 2: Read / starred mis-stored as labels

Read state and starred state are message state per the glossary, but several helpers inject them into label lists and persist them as `thread_labels` rows.

- **`crates/common/src/label_flags.rs:15`** - `assemble_labels()` creates synthetic `"UNREAD"` and `"STARRED"` label IDs from booleans and returns a flat `Vec<String>` mixing them with folder and tag IDs. Root of the projection.
- **Provider mappers feed read/starred through that path.** `crates/graph/src/folder_mapper.rs:93`, `crates/jmap/src/mailbox_mapper.rs:42`, and `crates/imap/src/folder_mapper.rs:55` all push provider read/starred state into the label list. `crates/imap/src/folder_mapper.rs:62-72` additionally pushes `DRAFT` into IMAP message label lists.
- **Core mirrors thread booleans into `thread_labels` rows.** `crates/db/src/db/queries_extra/provider_sync_writes.rs:84` writes `"UNREAD"` / `"STARRED"` rows into `thread_labels` from the booleans, double-tracking state and creating sync churn.
- **`crates/provider-sync/src/graph/sync/folders.rs:83-105`** - Persists a synthetic `UNREAD` label row with `label_kind = 'tag'` into the `labels` table. UNREAD is message state and must not exist as a tag at all.
- **For Gmail, `UNREAD` can become a container row.** `crates/provider-sync/src/gmail/sync/labels.rs:31` treats it as a system label and routes it through the container path, so the same primitive is mis-classified in two different directions.
- **UI consumer surfaces UNREAD as a user label.** `crates/core/src/db/queries_extra/navigation.rs:290` returns it in the account labels list, `crates/app/src/ui/sidebar/labels.rs:22` renders it in the LABELS section, and `crates/app/src/ui/reading_pane.rs:744` includes it in the label-pill row.
- **`crates/provider-sync/src/imap/sync_pipeline.rs:684`** - Helper `sync_thread_read_starred_labels` writes read/starred via labels; the name documents the divergence.
- **`crates/imap/src/folder_mapper.rs:140-146`** - Test `test_get_labels_for_message` asserts the broken UNREAD/STARRED-as-label behaviour, locking the drift in.

**Severity: High.** Read/starred state is double-tracked between message-state columns and label rows, which means sync churn and inconsistent UI counts.

---

## Theme 3: Reserved RFC 5788 system keywords surfaced as user labels

The glossary classifies `$Forwarded`, `$MDNSent`, `$Junk`, `$NotJunk`, `$Phishing` as message state or reserved; they must not appear in the LABELS section.

- **IMAP collects keywords without filtering, then sync stores them as user labels.** `crates/imap/src/client/commands.rs:455` reads every custom keyword from the wire without filtering reserved ones. `crates/provider-sync/src/imap/sync_pipeline.rs:687-734` then upserts each as `label_kind = 'tag'`. `$Forwarded` goes into the LABELS section instead of routing to `is_forwarded`; the other RFC 5788 keywords go straight through.
- **JMAP filters them out correctly but discards them silently.** `crates/jmap/src/parse.rs:108-112` drops every `$`-prefixed keyword. The filter is right for labels, but `$answered` / `$forwarded` should be routed to message state, not just dropped.
- **No filter anywhere in `crates/{common,db,core,types}`.** Workspace grep for `$Forwarded` / `$MDNSent` / `$Junk` / `$NotJunk` / `$Phishing` returns zero hits in scope. The filter must be added at sync time and as a defensive read-time filter in `get_labels`.

**Severity: High.** Visible to any IMAP user whose server applies these keywords.

---

## Theme 4: Graph importance + Outlook-specific synthesis missing

- **No "High importance" / "Low importance" labels synthesised.** Graph requests `importance` in the API select clause at `crates/graph/src/types.rs:216`, but `GraphMessage` has no `importance` field (`crates/graph/src/types.rs:13`), so the wire value is parsed and discarded. `crates/graph/src/parse.rs:78` only derives read/starred, folder, categories, and flag labels.
- **`add_tag`/`remove_tag` patch Graph `categories` only.** `crates/graph/src/ops/mod.rs:150` has no special-case for the synthesised importance labels - even if they existed, applying or removing one wouldn't update Graph's `importance` field.
- **`FOCUSED` label leaked from Outlook Focused Inbox.** `crates/graph/src/parse.rs:101-107` synthesises a `FOCUSED` label from `inferenceClassification`. The glossary has no entry for this; Focused Inbox classification should either map to a documented Ratatoskr concept or be dropped.
- **Graph `flag.flagStatus` -> STARRED mapping is unverified end-to-end.** Action D's audit did not find a divergence in the action pipeline, but no audit traced the Graph parse path that should populate `is_starred` from the follow-up flag.

**Severity: Medium.** Importance is a feature gap (read + write); FOCUSED is an unauthorised surface that lands in user-visible label lists.

---

## Theme 5: Command palette mixes folders and labels

The palette's "move to folder" and "apply label" actions both read from a query that returns either, with no `label_kind` filter.

- **`crates/db/src/db/queries_extra/command_palette.rs:23`** - `get_user_labels_for_account_sync` selects every visible non-system row from the `labels` table with no `label_kind` predicate, so the result is a mixed list of folders and tags.
- **`crates/core/src/command_palette_queries.rs:6`** - Core exposes that same result for both folder and label palette calls.
- **`crates/app/src/db/palette.rs:17`** - The app explicitly treats labels as folders here, so "move to folder" can offer tags as move targets and "add label" can offer folders as label candidates. Both are wrong by glossary semantics.

**Severity: High.** Visible to anyone using the palette: a user can attempt to "move" a thread to a tag (which probably fails silently or applies the tag) or "apply" a folder as a label (likely renders the same way to the user but routes through the wrong code path).

---

## Theme 6: IMAP `\Flagged` classified as a folder, not message state

The glossary classifies `\Flagged` as starred message state. Two places treat it as a system folder.

- **`crates/db/src/db/folder_roles.rs:91`** - Folder role mapping treats `\Flagged` (and starred aliases) as the `STARRED` system folder.
- **`crates/imap/src/folder_mapper.rs:27`** - IMAP folder mapping converts the provider primitive into a system folder row.

The Ratatoskr `STARRED` universal sidebar item is meant to be a predicate-based virtual view filtering on the `is_starred` boolean (per the glossary's Universal Folders section), not a real folder backed by an IMAP mailbox or label row. Treating IMAP's per-message `\Flagged` flag as a folder means starred state moves through the folder pipeline when it should move through the message-state pipeline.

**Severity: Medium.** End-to-end behaviour might still come out right because the `STARRED` ID is canonical and both pipelines end up at the same DB rows, but it's a routing error that will bite the moment anyone touches the system-folder mapping.

---

## Theme 7: Typed ID names use "Tag", which the glossary forbids

`Tag` is on the glossary's "Terms NOT used" list. The data layer is full of it.

- **`crates/types/src/typed_ids.rs:13-17`** - `TagId` should be `LabelId`. Doc-comment also reads "tag/keyword/category", which entrenches the wrong vocabulary.
- **`crates/types/src/sidebar_selection.rs:18-19`** - `SidebarSelection::Tag(TagId)` is the worst offender: a user-facing enum variant named with a forbidden term. Should be `Label(LabelId)`.
- **`crates/common/src/ops.rs:61-72`** - `ProviderOps::add_tag` / `remove_tag` taking `&TagId`. These are the canonical action-pipeline trait methods; "tag" leaks from here into every provider implementation. Should be `add_label` / `remove_label` with `&LabelId`.
- **Action pipeline already calls them with `label_*` names internally** (per Agent D), so the trait wrapper is the only seam where the term escapes.

**Severity: Medium.** Naming-only, but pervasive and reaches into the user-facing sidebar enum.

---

## Theme 8: Folder vs label naming drift in provider mappers

Several mapper structs name folder-only data as "label".

- **`crates/jmap/src/mailbox_mapper.rs:5-9, 15-34`** - `MailboxLabelMapping`, `map_mailbox_to_label` describe folder mappings.
- **`crates/graph/src/folder_mapper.rs:17-24, 84-99`** - `FolderLabelMapping`, `label_id`, `label_name`, `label_type`, `parent_label_id`, `get_labels_for_message`, `resolve_folder_id` returning a `label_id` - the values are folder IDs.
- **`crates/imap/src/folder_mapper.rs:5-10, 28, 56`** - Same pattern: `FolderLabelMapping`, `label_id`, `label_name`, `label_type`, `get_labels_for_message` for folders.
- **`crates/graph/src/parse.rs:90-108` + ParsedMessageBase.label_ids** - Variable `label_ids` holds a mixed list of folder IDs, `cat:` labels, and synthesised state strings. Same variable name in `crates/gmail/src/parse.rs:98-104` and `crates/jmap/src/parse.rs:115`.

**Severity: Medium.** Naming-only, but every reader of the provider crates has to mentally translate.

---

## Theme 9: Action pipeline carries "Label" naming for folder operations

- **`crates/service/src/actions/move_to_folder.rs:19, 25, 46, 51, 93, 98, 148`** - Parameter `source_label_id: Option<&FolderId>` is named "label" but holds a folder ID. Same drift in JSON params key `sourceLabelId` (lines 51, 98). The source of a move is, by definition, a folder.
- **`crates/service/src/actions/pending.rs:51, 78, 98, 239, 327`** + **`crates/service/src/actions/batch.rs:429`** - The pending-ops journal/replay also carries `sourceLabelId`. Renaming requires a wire-format migration since the value is persisted across restarts.
- **`crates/service-api/src/request.rs:303, 310`** - `TestDbLabelRow.parent_label_id: Option<String>` describes a folder's parent (folders nest, labels do not). Same shape mirrored at `crates/service/src/handlers/test_helpers.rs:1269, 1286, 1311`.
- **`crates/service/src/actions/folder.rs:43-46, 91, 135-138, 179, 223`** - `create_folder` / `rename_folder` / `delete_folder` take `folder_id: &str` and `parent_id: Option<&str>` instead of `&FolderId` / `Option<&FolderId>`; the trait they call already uses typed IDs.
- **Command context uses `ViewType::Label` for both folders and tags.** `crates/cmdk/src/context.rs:26` and `crates/app/src/command_dispatch.rs:56` carry `current_label_id` and `ViewType::Label` for the active sidebar scope - whether it's a folder or a tag.

**Severity: Medium.** Includes a wire-format migration if the pending-ops journal field gets renamed.

---

## Theme 10: `upsert_label` defaults silently to `'container'`

- **`crates/db/src/db/queries.rs:269-297`** - `upsert_label` omits `label_kind` from its INSERT and ON CONFLICT clauses, so inserts via this path default to `'container'`. User labels written via this helper become folders. The parallel writer at `crates/db/src/db/queries_extra/label_persistence.rs:28-87` does set `label_kind`; only the legacy helper has the bug.

**Severity: High (latent).** Any caller of `upsert_label` is misclassifying user labels as folders. The fact that the issue hasn't surfaced suggests this helper has no current callers, but it's a foot-gun.

---

## Theme 11: `get_labels` returns folders + tags + reserved keywords with no filter

- **`crates/db/src/db/queries.rs:44-50`** - `get_labels(conn, account_id)` returns every row in `labels` for the account, mixing `'container'` and `'tag'` and any system-keyword rows that snuck in. Every caller treats the result uniformly. Specifically: the reading pane's tag pill filter at `crates/app/src/ui/reading_pane.rs:744-745` filters by `label_kind == "tag"` but has no reserved-keyword exclusion, so RFC 5788 system keywords appear as pills.

**Severity: Medium.** Defensive filtering should live in this helper.

---

## Theme 12: UI gaps and naming drift

- **No label dots in the thread list.** `crates/app/src/ui/thread_list.rs:658, 676` hard-codes `let label_colors: &[(Color,)] = &[]` and passes it to every `thread_card`. Threads render no label indicators at all, contradicting the "labels always carry a coloured dot" contract.
- **`ThreadListMode::Folder` covers both folders and labels.** `crates/app/src/ui/thread_list.rs:151-152, 189, 524, 578` doc-comments "Browsing a folder or label." Variant name violates the glossary's identifier rule; rename to `Container` / `Scope` or split into two variants.
- **Empty-state copy says "folder" for labels.** `crates/app/src/ui/thread_list.rs:524` hard-codes `"This folder is empty"` for every non-search mode, including label scopes.
- **`palette.rs` overloads "label" for iced widget captions.** `crates/app/src/ui/palette.rs:94, 174-177, 195, 202, 227, 431, 506-507, 525, 550, 564-581` use `stage2_label`, `param_label`, `label_style`, `label_text`, `label_str` for button captions / placeholder text. The glossary's identifier rule applies: `label` should mean tag. Rename caption-style identifiers to `caption` / `placeholder` / `display_text`.
- **`TypeaheadItem.label: String`** at `crates/app/src/ui/thread_list.rs:30` is the same caption-vs-tag overload.
- **`reading_pane.rs:744-745` comment** mentions "folder/container labels" - phrase mixes terms; folders are not "folder labels".

**Severity: Mixed.** The thread-list missing dots and the empty-state copy are user-visible (High); the rest are Medium / Low naming drift.

---

## Theme 13: Schema dead column + glossary path drift

- **`crates/db/src/db/types.rs:121-152` + `02_mail.sql:7, 16`** - `DbLabel.label_type: Option<String>` (column `type`) appears to be a dead discriminator alongside `label_kind`. Present in schema, struct, and both upsert writers but with no documented consumer. Also flows through `LabelWriteRow` (`crates/db/src/db/queries_extra/label_persistence.rs:4-26`).
- **Glossary path drift.** `docs/glossary/folders-labels.md` line ~141 advertises `crates/common/src/folder_roles.rs` as the home for `SYSTEM_FOLDER_ROLES`. Actual file is `crates/db/src/db/folder_roles.rs`, re-exported via `crates/common/src/lib.rs:7`. Either move the file or update the glossary path.
- **`scoped_queries.rs` constant naming.** `LABEL_FOLDER_IDS` at `crates/db/src/db/queries_extra/scoped_queries.rs:310-312` and `get_label_folder_unread_counts` at `:359-393` use "label folder" together for what are pure system folder IDs. Rename to `SYSTEM_FOLDER_IDS` / `get_system_folder_unread_counts`.

**Severity: Low.** Cleanup, no behavioural impact.

---

## Theme 14: Doc-comment hygiene

Comments that codify the wrong model:

- **`crates/graph/src/parse.rs:47`** - "Labels derived from folder + categories + read/starred flags". Wrong model.
- **`crates/jmap/src/mailbox_mapper.rs:11-14, 42-43`** + **`crates/imap/src/folder_mapper.rs:12, 28`** + **`crates/graph/src/folder_mapper.rs:8`** - Doc comments saying "Gmail-style label ID" for what is a folder ID.
- **`crates/service/src/actions/label.rs:16, 96, 134, 156, 238, 278`** - Doc comments say "Container labels (folders) are rejected" - phrase "container labels" blends terms.
- **`crates/service/src/actions/folder.rs:196`** - Comment refers to `parent_label_id` for a folder's parent.

**Severity: Low.** Comments only.

---

## Verified correct (no divergence)

Some surfaces audit clean and should be cited as the reference shape:

- `MailOperation` / `WireMailOperation` variants (`AddLabel` / `RemoveLabel` for tags; `MoveToFolder` for folders; `SetStarred` / `SetRead` for message state) match the glossary's Operations section.
- `WireFolderId` / `WireTagId` mirror typed IDs across the wire (although `TagId` needs to become `LabelId` per Theme 5).
- `$Forwarded` / `$MDNSent` etc. do not surface as user-applyable labels via `AddLabel`; `mark_mdn_sent` is a dedicated `ProviderOps` method.
- Star action routes through `provider.star()` (dedicated message-state primitive), not via a category/label apply.
- `Trash` / `Spam` / `Archive` / `MoveToFolder` are dispatched as Move semantics, not Apply/Remove.
- JMAP parser correctly filters `$`-prefixed keywords from `keyword_categories` (the issue is what happens after, see Theme 1).

---

## Audit status

Outside review merged. All five audits are reflected above; the convergence on Themes 1-4 (the four highest-severity findings) is unanimous across all five sources.
