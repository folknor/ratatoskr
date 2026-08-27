# TODO

As a general rule, TODO.md items are **removed** when completed.

## Remaining Work

### Bifrost / saehrimnir follow-ups

Each is a side-quest per `docs/side-quests.md`, not a ratatoskr change.

- [ ] **bifrost `LiveSupersedes` is deliberately dead, and a wiring fix is UNSOUND** - Read this before touching the set. It is constructed and threaded onto the backfill handle, nothing calls `.add()`, and `filter_supersedes` therefore filters nothing, so cold-start backfill can double-emit an object the live `changes_stream` already announced. That much is the original finding. What was learned since is why the obvious fix is worse than the bug:
  - **Broadcast delivery is not proof of receipt, so there is no correct moment to record an id.** `send` returning `Ok` means nothing: a sentinel receiver keeps it `Ok` with no consumer attached, and a late subscriber starts at the ring tail. "A real receiver existed at send time" is no better - `tokio::broadcast` overwrites its ring and a lagging receiver silently skips values, and `bifrost-sync` has no `Lagged`/`RecvError` handling anywhere to detect or replay that. Recording on send therefore lets backfill suppress the ONLY copy a consumer could ever receive, which is silent data loss - strictly worse than the duplicate the set exists to prevent.
  - **Atomicity alone does not rescue it.** The window between `filter_supersedes` and `tx.send` in `backfill/runner.rs::run_partition` contains no `.await`, but the backfill walk and the live feed are separate spawned tasks running genuinely in parallel. A mutex makes each `add`/`take` atomic and does nothing for the compound take-then-publish, so a stale `Created` can still be published after a live `Destroyed`, resurrecting a deleted object.
  - **The only sound trigger is the consumer ack**, which this crate already treats as proof of receipt precisely because broadcast delivery is not. A correct design buffers ids as pending keyed by their batch checkpoint, promotes them on ack, needs a policy for batches carrying no checkpoint (never promotable) and a bound on the pending buffer, AND needs take-and-publish to be one indivisible decision on both sides. That is a redesign of the ack path, not a wiring fix.
  - **The payoff does not currently justify that cost**, which is why it was reverted rather than attempted: the only prize is avoiding a duplicate `Created` that consumers must already tolerate, since a crash mid-plan re-walks every partition and re-emits acked pages. It also cannot be gated black-box - the window has no scheduling point, so proving it needs a test-only seam in the client, and the `Destroyed` branch has no gate available at all (see the stale-inventory item below).
  - A tripwire exists: `an_unpopulated_set_forwards_every_inventory_entry` fails the moment anyone starts populating the set, which is the moment to re-read this. The O(1) `take` and its tombstone-safety tests were kept and are independently correct.

- [x] **JMAP `Email/set` can still empty `mailbox_ids`** - Resolved as a side-quest (saehrimnir `e040fa9`, promoted + mock reinstalled). `apply_email_patch` - the single funnel for `Email/set` update, `EmailSubmission/set`'s `onSuccessUpdateEmail`, and the change-script `email_update` op - now rejects any patch that would net out to an empty `mailboxIds` with a per-item `invalidProperties` SetError (checked once after the whole patch folds in, so drop-one-add-one still applies; no partial mutation, no transition). Create and `Email/import` refuse the same shape, `Set` parsing is RFC-conforming on both properties (`true`-valued keys only; per-key `false` = removal alongside `null`), and the deliberate Gmail contrast (same shape = archive, because Gmail really has All Mail) is documented at both decision sites with do-not-unify notes. Seven mock-side tests plus the ratatoskr gate `jmap-email-set-empty-mailboxids-rejected.lua`, which drives all three refused shapes at the promoted mock, proves a resync changes nothing on our side, and pins that a legal membership replace still applies and reaches our DB. The jmap sync family (email-set-delta, action/send writeback, scheduled-send, incremental-steps, mdn, initial, steady-state) re-verified green against the reinstalled binary. Lateral findings from the same pass are the item below.

- [ ] **saehrimnir JMAP mock-fidelity gaps (lateral findings from the `Email/set` side-quest, none fixed)** - Each is a small future side-quest; none has a consumer-side gate depending on it today. (a) `Email/set` UPDATE never validates that a mailbox id EXISTS - create and import reject unknown ids, but an update can place an email in a phantom mailbox that no listing ever serves; deliberately not fixed in the side-quest because RFC 8620 §5.3 allows create-references (`mailboxIds/#ref`) in patch paths, which naive validation would break - wants its own slice with a back-reference decision first. (b) `EmailSubmission/set`'s `onSuccessUpdateEmail` branch silently swallows a rejected patch (`if apply_email_patch(..).is_ok()`) - post-fix that is safe (it can no longer orphan an email) but a typo'd fixture gets a green submission with an unmoved email and no diagnostic. (c) No `set_error` call site emits the RFC-suggested `properties` array on `invalidProperties` SetErrors, and the helper is triplicated across `jmap.rs` / `jmap_calendar.rs` / `jmap_contacts.rs`, so a shape change is three edits.

- [ ] **saehrimnir has no stale-inventory affordance, so a retracted-id race cannot be staged** - The mock's listing is live: `email_destroy` removes the row, so no later inventory page ever offers a destroyed id and a consumer's supersedes `take()` is never called for it. Exercising the `Destroyed` half of any de-dup design needs a destroyed object to KEEP appearing in listings for a bounded window after the change feed retracted it - real eventually-consistent server behaviour, buildable on the existing change-script machinery. Deliberately not built: no gate consumes it yet. `backfill-late-tombstone.lua` is the ordering fixture that DOES work (deliver, then retract, last-writer-wins) and is named so it cannot be mistaken for the race it does not stage.

- [ ] **saehrimnir `announce` makes an interleaving reachable, not certain** - The trigger places a change and its push at a chosen point in a walk, but determinism ends at the socket: a Gmail push carries only `{emailAddress, historyId}`, so the consumer does a follow-up `history.list` round trip before it can record anything, and whether that completes before the walk reaches the same id is consumer task scheduling. Making an interleaving certain would need a request-dependency barrier ("hold the page-N request until `history.list` has been served at least once since the announcement"), which sits naturally on the same trigger machinery. Not built.

- [x] **Consume saehrimnir's fixture identity digest** - Resolved, but NOT via the mechanism the item proposed, which turned out to be a tautology: brokkr feeds the spawned mock OUR fixture file (`[ratatoskr] fixtures_dir = crates/app/tests/sync-fixtures`), so the running mock's `/test/fixture/identity` digest is always computed over the very copy we would compare it to. The drift pair that actually matters is our copy vs the saehrimnir REPO's copy (which the mock's own tests pin behavior against). `crates/app/tests/shared_fixture_identity.rs` now byte-compares the two through the in-tree `./research/saehrimnir` clone, data-driven over a `SHARED_FIXTURES` list for future shared fixtures, skipping on hosts without the research clone (drift cannot be introduced where only one tree exists). The identity endpoint remains useful for manual `sha256sum` reproduction; nothing gates on it.

- [ ] **`DebtLedger` cannot be persisted, so `SqliteCheckpointStore` holds it in memory** - bifrost's `CheckpointStore` rework made `apply_transition` the only mutating operation, taking a checkpoint and the account's whole `DebtLedger` as one atomic write. Our SQLite backend can honour that for the checkpoint half only: `DebtLedger`'s fields are private and `bifrost-sync` exposes no encode/decode pair for it the way it does for `Checkpoint` (`encode_envelope` / `decode_envelope`), so there is nothing a backend can serialize. The store therefore keeps a process-lifetime `HashMap<AccountId, DebtLedger>` alongside the durable rows, written only after the checkpoint commits. The degradation is one-directional and documented at the field: a restart forgets accepted debt, so a degraded scope comes back looking clean until something re-enumerates it and re-raises the obligations - it never invents coverage, because an empty ledger permits completion only for a walk that itself reported complete. Making it durable is a bifrost-side change (give `DebtLedger` a serialization surface), then a table on ours.

- [ ] **The synthetic broadcast-lag warning has no structural marker, so we match on its message text** - `account_changes_stream` now returns `ChangesReceiver`, which converts ring overflow into a synthetic account-scoped `Warning::OperatorAttentionNeeded` instead of surfacing `RecvError::Lagged`. The consumer still needs to know a lag happened (it detaches and re-drives so the next attach reconciles from the last durable checkpoint, and the harness gates assert on `report.lagged`), but degraded coverage raises the same `WarningKind` on the same scope, so nothing distinguishes the two except the message prefix. `is_lag_warning` in `consumer/mod.rs` is deliberately the single place that knows this. Ask bifrost for a real marker - a `WarningKind` of its own, or a flag on `MultiplexerEvent` - and that function is the one thing to rewrite.

### Ratatoskr follow-ups from the migration

- [ ] **Retire the ordinal-position attachment pairing in `hydrate.rs` now that `MessageAttachment` carries the metadata** - Deliberately deferred while catching ratatoskr up to bifrost's current API; the landed fix is behaviour-preserving on purpose, because it rode in on a chrono-to-jiff migration and changing attachment semantics in the same pass would have made any regression unattributable. Bifrost's `d585526` ("types: ship the shared inbound MIME parser") reshaped `Message::attachments` from `Vec<BlobHandle>` to `Vec<MessageAttachment>`, moving the blob id inside `source: AttachmentSource::{Blob(BlobHandle), Inline(Bytes), None}` and adding `filename` / `content_id` / `inline` / `truncated` as first-class fields. Those are exactly the three values `build_consumer_row` currently recovers by re-parsing the verbatim RFC822 and pairing the parsed parts against the structured list **by ordinal position** - a heuristic whose own comment ("the structured `BlobHandle` cannot carry the part name, the Content-ID, or the inline disposition") is now false. The work: read the fields off `MessageAttachment`, delete the ordinal pairing and the `parsed_attachments` plumbing that feeds it. Two things make this more than a cleanup:
  - **It moves golden-test output.** `consumer/golden_test.rs` is byte-identical-by-construction against the production merge, so this is a deliberate fixture rebase, not an incidental diff. Per the behavioural gate in AGENTS.md a compile-only replacement is under-gated here; it wants the `bifrost-consumer-*` scripts re-run, not just `brokkr check`.
  - **The `Inline` / `None` variants are an unresolved correctness question, not just a match arm.** The old code assumed every attachment was blob-backed and keyed `attachments.content_hash` by blob id, which is what links a row into the attachment-store dedup. An `AttachmentSource::Inline(Bytes)` attachment has no blob id, so that key does not exist for it - hashing the carried bytes instead is a new code path, and guessing wrong silently corrupts dedup (a hard requirement per AGENTS.md) at runtime rather than at compile time. The interim fix handles `Blob` exactly as before and log-and-skips the other two variants, so the gap is visible in logs; settle whether the four mail protocols ever actually return `Inline` before building the byte-hashing path.

- [ ] **The sync-harness red is now much wider than the four gates below, and the JMAP family looks like a bifrost change-stream spin** - Measured on the dependency-bump / bifrost catch-up pass with a green `brokkr check` (2326 tests). `brokkr sync --all` fails roughly 45 of 163: every `jmap-*` mail script (the calendar ones mostly pass), plus a scattering of `gmail-mdn`, `gmail-push-pubsub`, `gmail-send-writeback`, `gmail-incremental-steps`, `gmail-thread-partial-delta`, `graph-mdn`, `graph-push-webhook`, `graph-send-writeback`, `imap-draft-discard`, `imap-incremental-new-change`, `imap-push-idle`, `imap-jmap-shared-state`, and the three CardDAV contact gates already itemised below.

  The JMAP failures share one signature. `jmap-initial` runs against a **two-email** fixture, and `service.log` shows the consumer reporting a broadcast lag ten times in a row before the account gives up; the account never reaches caught-up and the script's initial `start_sync` fails at ~31s. Two emails cannot overflow a broadcast ring, so this is not volume - it reads as the engine republishing in a loop. Confirmed NOT to be the consumer's lag handling: with the lag arm disabled the script stops failing at 31s and instead hangs to the 120s ceiling, which is the same non-convergence with the bail removed. The mock starts, loads the fixture, serves, and shuts down cleanly, and its CardDAV/JMAP servers log no errors.

  Two things to rule out before digging into bifrost. (a) The harness runs an INSTALLED `/home/folk/.cargo/bin/saehrimnir`, and the failing set is concentrated in push, MDN, and send-writeback - exactly what a binary stale against the sibling repo would break. Reinstall it and re-run before believing any of the rest. (b) If the red survives that, it is a bifrost side-quest per `docs/side-quests.md`, not a ratatoskr change: the consumer side compiles and its own `bifrost-consumer-*` gates all pass.

- [ ] **Four sync-harness gates are red, and both causes are ratatoskr not finishing the bifrost foreign-namespace catch-up** - `jmap-shared-account-sync`, `contacts/carddav_pull`, `contacts/reconcile_deleteall`, `contacts/writeback_carddav`. Superseded in scope by the item above, but the per-cause analysis here still stands. Confirmed pre-existing (they fail identically with the jiff migration stashed), and they cannot have passed recently: the workspace did not compile at all before `cab5f7e4`, so they have been red since bifrost's foreign-namespace work landed and ratatoskr fell behind. `cab5f7e4` made the code compile against the new API; it did not re-implement the consumer side of the reshaped contract. Two distinct causes:

  **(a) `jmap-shared-account-sync` - ratatoskr does not understand bifrost's owner-qualified scope ids.** The foreign scope DOES arrive; the consumer drops it. `service.log` shows, three times per run:
  `WARN service::bifrost::consumer - skipping un-attributed bifrost scope Folder(FolderId("account-team\u{1f}")); it will be redelivered`
  That `\u{1f}` is bifrost's owner-qualification separator from `77e7a06` ("jmap: owner-qualify foreign thread ids and make the v2 reseed real") - the id is `owner<US>local` with an empty local part for the foreign account's own scope. `ChangeStreamConsumer::resolve_attribution` resolves a scope only by matching it against container native ids in `ContainerIndex`, and a grep for `\x1f` across `crates/service/src` finds it ONLY in `checkpoint_store` test fixtures: ratatoskr has no knowledge of the encoding anywhere in the attribution path. So the scope never matches, attribution is `None`, every foreign batch is skipped "and redelivered" forever, no foreign mail is persisted, and `reconcile_namespace_registry` never sees a `Shared`-namespace container to register - which is why the script fails at its FIRST foreign assertion (`foreign message missing`, line 31) rather than at the later `owner_email` principals gate. Fix shape: teach the attribution path to split the owner-qualified form and attribute by OWNER, rather than requiring an exact container-id match. The one-refresh-per-scope latch in `resolve_attribution` means this fails quietly rather than hot-looping, which is presumably why it went unnoticed.

  **(b) The three CardDAV contact gates - `address_books_list` answers `Unsupported`, so no wire call is ever made.** `test.contact_pull` returns `{"imported":0}` with `outcome=ok`, and the mock's CardDAV server logs ZERO requests despite starting and loading the fixture. `run_contact_pull` treats `RecoveryClass::Unsupported` from `address_books_list` as `Ok(0)` (a deliberate contract for providers without contacts), so the failure is silent by design. The seed is correct - the `test.seed_account` frame carries `caldav_url`, `caldav_username`, and `caldav_password`, and `build_imap_factory` composes `CardDavConfig` onto the IMAP account when all three are present - so the capability is being answered `false` despite the composition being configured. Not chased further: the next step is inside bifrost's IMAP+CardDAV composition (whether `pim_methods.address_books_list` is set when a `CardDavConfig` is attached), which needs a look at the bifrost side rather than ours.

- [ ] **`canonical_recurrence_id` mints a wrong key for offset-bearing RECURRENCE-IDs (pre-existing, preserved bug-for-bug through the jiff migration)** - `crates/calendar/src/idmap.rs`. The RFC 3339 arm formats the OFFSET-LOCAL wall clock and appends a literal `Z`, so `2026-03-15T10:00:00+01:00` canonicalises to `20260315T100000Z` instead of the true UTC `20260315T090000Z`. The `Z` asserts UTC, so the key is simply wrong for any provider that sends a non-zero numeric offset - and it will not match the key `db`'s `canonical_recurrence_slot` mints for the same instant during master expansion, which is exactly the comparison the value exists for (a missed match means a phantom override renders alongside the instance it was supposed to replace). Not fixed during the migration because the value is PERSISTED as `events.recurrence_id_canonical` and compared against keys minted by earlier syncs: correcting the derivation changes stored keys, so it is a data migration (re-derive on read, or a one-shot backfill + forced calendar resync) rather than a formatting change. The chrono behaviour is reproduced deliberately and documented at the call site. Providers that send `Z` or a zoneless wall clock are unaffected, which is probably why this has never been noticed.

- [ ] **Several UI timestamp labels render in UTC instead of the host zone (pre-existing, preserved bug-for-bug through the jiff migration)** - Surfaced by reading every `chrono::DateTime::from_timestamp` call site during the migration. `from_timestamp` yields a `DateTime<Utc>`, and these sites formatted it directly without a `with_timezone(&Local)`, unlike their neighbours:
  - `ui/widgets/cards.rs` - thread and message card timestamps (relative "%-I:%M %p" / "%a" / "%b %d" labels, the absolute "%b %d, %Y" forms, and the `+Nd` offset in `DateDisplay::RelativeOffset`).
  - `ui/widgets/attachment.rs` - the "%b %d" date in all three attachment meta lines.
  - `pop_out/message_view.rs::format_date` - the message header date.
  - `ui/calendar_time_grid.rs::format_event_time` - the event block's time LABEL, which is the sharpest case: `event_minutes` positions the same block using the HOST zone, so a block can be drawn at its local time and labelled with its UTC time. The module comment there explicitly says "Display in local time everywhere."
  - `ui/right_sidebar.rs::events_for_date` and `ui/calendar_month.rs::build_month_grid` derive an event's day span in UTC, while `CalendarState::rebuild_view_data` derives the same span in the host zone - so a late-evening event can land on different grid days depending on which path rendered it.

  For a user at UTC+0 none of this is visible, which is presumably why it survived. For the project's Norwegian users (UTC+1/+2) every affected label is off by an hour or two, and near midnight by a whole day. Not fixed during the migration because each one changes what users see, and a library swap is the wrong commit to hide that in. Fixing is mostly mechanical (`TimeZone::UTC` -> `TimeZone::system()` at the marked call sites, each of which carries a NOTE comment) but wants a decision on the calendar day-span question first, since that one is a real semantic choice rather than an oversight.

- [ ] **jiff migration: the `Z`-suffix / zoneless parse asymmetry is not compile-caught** - Recorded during the chrono-to-jiff migration, relayed from the bifrost side where it bit at every such site. `jiff::civil::DateTime` REFUSES a trailing `Z` (Temporal reads `Z` as an unknown offset, not as UTC), and `jiff::Timestamp` refuses a bare zoneless datetime - so the two failure modes are complementary and neither is caught by the type system. Every former `chrono::DateTime::parse_from_rfc3339` + `naive_local` site needs the try-civil-then-fall-back-to-`Timestamp`-at-UTC shape. This fails at RUNTIME on real provider data, which means a green `brokkr check` proves nothing about it: the affected paths (iCal `DTSTART`/`DTEND` parsing in `core/src/caldav/`, Graph and JMAP date fields, the smart-folder date parser) need harness coverage, not unit compilation. Sweep every ex-`parse_from_rfc3339` call site once the migration lands and confirm each one is exercised by a sync-harness script that feeds it both shapes.

- [x] **Delete `gmail_source_detach` in favour of bifrost's `bulk_move_from`** - Resolved. `run_container_move_plan` now dispatches one `bulk_move_from` campaign; the plan carries the resolved source UNFILTERED for every provider (the exclusions `gmail_source_detach` applied - Gmail-only, skip INBOX, skip source == destination - are bifrost's `move_patch` semantics now, and the folder-model providers' default impl ignores the source entirely), so the workaround and its per-id `remove_from_container` detach are deleted. `detach_from_container` survives only for the Gmail ARCHIVE shape, which has no destination to ride `bulk_move_from`. The read-back caveat the item flagged is gated on our side as the item asked: new `gmail-move-source-detach.lua` (+ `gmail-move-sources.toml`) drives restore-from-trash and un-spam and asserts BOTH the source label is gone after resync AND each move was exactly one `batchModify` with zero per-id message mutations.

- [x] **Bulk star never coalesces** - Resolved. Star now rides `bulk_set_flags` like read state, so an N-thread star campaign is ONE bulk campaign instead of one single-object call per message. (Not one request: every provider chunks the id set - Gmail by its `batchModify` limit, Graph by `max_items`, JMAP by `maxObjectsInSet`, IMAP by folder grouping. The win is the native batch verb plus one pass through the engine's idempotency / read-back / recovery pipeline.) The `StarredFlagShape` capability dispatch bifrost's per-message `set_starred` performed is reproduced by the flag string alone (`dispatch_target::starred_flag`): every provider's bulk flag path already canonicalizes `\Flagged` into its native star field (Gmail -> `STARRED` label, Graph -> `flag.flagStatus`, IMAP -> the `\Flagged` system flag), and JMAP takes the verbatim `$flagged` keyword. IMAP's bulk path parses ONLY the backslash form - `$flagged` would degrade to a custom keyword - which is why the per-provider string is load-bearing. Pinned against the consumer's read side (`hydrate::normalized_flags`).

- [x] **IMAP Archive from a non-inbox folder degrades to LocalOnly** - Resolved, and it was worse than recorded. B15 already deleted the singleton `add_to_container` + `remove_from_container` compose the original finding described; what survived was the destination-less arm of `dispatch_container_move`, reached whenever `role_target(Archive)` is unresolved. It composed a batch-wide `source = INBOX`, and bifrost lowers IMAP `remove_from_container` to `\Deleted` + `UID EXPUNGE` - so archiving from a custom folder failed `Unsupported` (the recorded symptom) while archiving from INBOX destroyed the message: immediately where UIDPLUS or IMAP4rev2 permit `UID EXPUNGE`, and otherwise by leaving it `\Deleted` and exposed to the next expunge from any client. The destination-less shape is now decided by the OPERATION, not by a missing destination (`is_gmail_archive_shape`), so a Trash or Spam role missing from the cached snapshot can no longer degrade into an archive - it forces the container refresh and then fails terminally. A non-Gmail archive with no Archive-role folder is a terminal not-found.

- [x] **Gmail moves left the source label attached** - Found by review of the item above, same code path. Bifrost's Gmail `move_patch` is destination-only (add the destination, remove INBOX) and a Gmail message id carries no source label, so unlike the folder-model providers Gmail cannot derive each message's source from the id set. Meanwhile `move_to_folder::move_local` / `spam::spam_local` DO drop the source locally, so a move out of TRASH or SPAM - or an un-spam, whose INBOX destination makes the patch remove nothing at all - came back from the next sync with the container the user moved away from restored. `gmail_source_detach` now supplies the explicit detach, Gmail-only and ordered after the move; `RemoteBatchKey::MoveToFolder` carries the source so moves out of different sources no longer coalesce into one wrong patch. Trash and mark-as-spam deliberately keep user labels, which is native Gmail behaviour and matches what the local writes do.

- [x] **Gmail multi-message-thread partial-delta sibling scenario is not integration-gated** - Gated by `crates/app/tests/sync-harness/gmail-thread-partial-delta.lua` against the `gmail-incremental.lua` fixture (two-message `thread-003`, step 4 stars `email-003a` alone). Asserts the sibling row survives, stays in the same thread, does NOT pick up the star (the mirror-image whole-thread-fan-out bug), and that the thread still rolls the star up and keeps its INBOX membership.

### Other

- [x] **`gmail-scheduled-send-rejected.lua` seeds the wrong provider string** - Fixed: `provider = "gmail_api"`. The gate passes again, so the capability rejection it exists to prove is exercised rather than short-circuited by `unsupported sync provider: gmail`.

- [x] **Pre-existing sync-harness failures: 8 `jmap-*` scripts and 6 `bifrost-consumer-*` frontmatter errors** - Resolved (all 14 pass targeted; full diagnosis in `.plans/sync-harness-red-triage.md`). Shape (b) was one cause as suspected, but in the scripts, not brokkr: the six inject-path `bifrost-consumer-*` scripts never had fixture frontmatter (they were B3a-infra service-harness gates - `brokkr service-test` then, `brokkr service` now - that also sit in `sync_script_dir`); they now carry a nominal `jmap-small.toml` like `hot-path` always did. Shape (a) was four distinct causes, only one a real product gap: (1) stale pre-B6 storage-model assertions (`bulk`/`many-folders` asserted `label_count` for JMAP mailboxes that correctly land in `folders` now); (2) the real gap - containers were only persisted at attach and on unknown-message-scope refresh, so a mailbox created/renamed server-side after attach (empty = no message batch) never reached `folders`; fixed by triggering `sync_containers` from the `Type(Mailbox)` change batches the consumer was already receiving and discarding (`is_container_change_batch`, empty caught-up pages excluded so idle cycles stay request-free); (3) `oauth-revoked-fails` seeded fabricated tokens, which bifrost's 401-refresh recovery converts into a working session via the mock's deliberate unknown-refresh fallback - restaged to mint-then-invalidate (the paired-refresh revocation cascade) and pin the `ReauthorizationRequired` classification instead of a raw 401; (4) stale wire/id shapes - `slow-paged` re-based onto bifrost's `maxObjectsInGet` paging (fixture raised to 2600 so the slow middle page actually exists), and `shared-blob` could not match attachment rows because sync-path ids are composite now, fixed by exposing `remote_attachment_id` in `TestDbAttachmentRow`. The follow-up residuals landed too: remote folder DELETION now reconciles (`reap_missing_personal_folders` in `containers.rs` - personal-namespace rows only, canonical/Gmail system ids and seeded rows excluded; gated by `jmap-mailbox-secondary-destroy-reap.lua` plus DB-backed unit pins), and `jmap-initial` / `imap-initial` now assert the specific `kw:project` label row instead of a `label_count` bound the harness seed alone could satisfy.

- [ ] **JMAP-Basic and CustomOIDC manual account setup write dead rows (pre-existing)** - Surfaced during B14 (account verify) but out of that item's scope, which excludes `account.create` changes. For `ManualProvider::Jmap` with password auth, `password_auth.rs::handle_submit_credentials` and `identity.rs::build_create_params` (~line 157) both send `jmap_url: None`, so `build_jmap_factory` fails `required_plain("jmap_url", ...)` with `MissingEndpoint` - the account can be neither verified nor synced. Separately, `CustomOidc{Imap,Jmap}` persist `resolved_provider = "oidc:{issuer}"` (`manual_config.rs:143`), which `MailProviderKind::parse` rejects, so those rows also fail the bifrost factory build (this wizard leg is gated/not-live today per the `oauth.rs` dead-code note). B14 verify correctly mirrors create for both (verify fails exactly where create writes a dead row - the intended verify/create parity), so no new divergence was introduced; the durable fix is to resolve/persist a real `jmap_url` on the JMAP-Basic create path and a parseable provider identity for CustomOIDC.

- [ ] **Sidebar scope persistence** - `selected_account` is in-memory state on the iced app model and resets to `None` (All Accounts) on every launch. The previous sidebar problem statement listed two options: persist to SQLite `settings`, or treat "All Accounts" as the launch default. Caution flagged in the original write-up: if persisted, the user needs strong visual context (account name/color in the sidebar header) so they don't fall into a "hidden mode trap" where they're scoped without realizing it and wonder where their email went. Decision deferred.

- [ ] **Settings/Notifications** - VIP Senders should move to contact editing, and this should be a toggle button here.

- [ ] **Custom colour picker for labels** - The per-account label editor and the label-group editor render `widgets::color_palette_grid` with a `+` tile at the end of the preset swatches. The tile dispatches `SettingsMessage::LabelEditorOpenCustomColor` / `SettingsMessage::LabelGroupEditorOpenCustomColor`, both currently no-op stubs in `crates/app/src/ui/settings/update/mod.rs`. Build the actual picker - hex input, sliders, or OS-native colour dialog - and wire it to write `(color_bg, color_fg)` into the respective editor state (and clear `color_index` since custom colours don't map to a preset). The account editor and add-account identity step deliberately pass `None` for `on_custom` and should keep doing so (account colours stay restricted to presets so the used-colour exclusion stays meaningful).

- [ ] **Settings/Accounts: Edit Account** - This section needs rework.

- [ ] **Password input UX** - `input_row_secure` currently masks every character to a dot the moment it's typed. Open questions: (1) should there be an "eye" toggle that reveals the value while held / pressed? (2) should the most recently typed character render as plaintext for ~1 second before turning into a dot, the way iOS / Android do? (3) should reveal-on-hover ever apply, or strictly explicit gesture? Affects `input_row_secure` in `row_widgets.rs` and every CalDAV / IMAP / SMTP password field that uses it.

- [ ] **Attachment saving** - Should remember last folder. Ideally last folder per thread ID.

- [ ] **Collapse individual expanded messages** - The button needs a new place to live - probably a very long, thin button that stretches across the entire horizontal space at the top of the message frame. This needs to be unified with the Attachments panel collapsing, which is currently taking up too much vertical space; also too much padding above the Attachments section.

- [ ] **Signatures: multi-account ownership** - Today a signature belongs to exactly one account (`signatures.account_id`), which makes "use the same signature on every account" tedious - users either duplicate (and then have to keep N copies in sync on every edit) or live without it. Generalize the model so a signature can be owned by a *set* of accounts: drop `signatures.account_id`, add a `signature_accounts(signature_id, account_id)` join table, and change the editor's Account row from a single-select to a multi-select. The two default toggles then become per-account: each member account gets its own "new-messages default" / "replies default" slot, preserving the "exactly one default per account" invariant the DB transactions already enforce. Update the description on the Account picker (`tabs/signatures.rs::signature_account_row`) once the model changes, since the current copy claims signatures are exclusive to one account.

- [ ] **Standardized popup/dropdown/modal** - Structural primitives are done (`modal_overlay`, `AnchoredOverlay`). The modal blocker now absorbs left/right/middle clicks, double clicks, and scroll so widgets behind the dimmed area no longer respond. The Add Account modal and confirm/form dialogs all share `ContainerClass::DialogCard` for visual consistency. Remaining gaps:
  - **Focus trapping** is still unsupported by iced (tracked separately below).
  - **Settings dropdowns** (the in-tab `select` widgets) close on outside click via their own `AnchoredOverlay::on_dismiss`, but that's per-widget rather than a unified contract; verify all `select` instances dismiss consistently.

- [ ] **Focus trapping for modals and sheets** - iced does not natively support focus trapping. Modal and Sheet surfaces should trap Tab/Shift-Tab focus within their content, but currently focus can escape to widgets behind the blocker. If iced adds focus trapping support, `modal_overlay()` is the single place to wire it in. Until then, this is a known contract gap.

- [ ] **Calendar event detail popover → AnchoredOverlay** - `calendar::popover_stack()` is the only anchored surface still using a hand-rolled `stack![]` instead of the `AnchoredOverlay` primitive. Target behavior: anchor near the clicked event pill using `anchor_point`. Requires capturing click coordinates in `CalendarPopover::EventDetail` (not currently stored).

- [ ] **Settings help tooltip → Ratatoskr Tooltip primitive** - The settings help surface uses `AnchoredOverlay` but is semantically a tooltip (hover-triggered, non-blocking, informational). The legacy pinned/sticky behavior has been removed. Should migrate to a Ratatoskr Tooltip primitive once one exists. Independent of the overlay standardization effort.

- [ ] **Escape key audit for overlay surfaces** - Calendar Escape now routes through `CalendarMessage::ClosePopover` / `CloseModal` instead of bluntly resetting the workflow, so Escape from the editor's ConfirmDiscard returns to the editor with the draft intact rather than nuking everything. Settings sheet's discard-changes confirm dialog also cancels on Escape. Still owed: a mechanical verification sweep over every Modal/Sheet surface (compose pop-out save-as-group dialog Escape, palette Escape inside a sub-state, add-account modal Escape, etc.) once everything has had some shakedown time.

- [ ] **Calendar move semantics for existing events** - The calendar picker is disabled for `EditingEvent` because moving an event between calendars requires provider-specific support (some providers need delete+create). When provider calendar-move APIs are implemented, re-enable the picker for existing events and update `account_id` ownership logic in the `CalendarSelected` handler accordingly.

- [ ] **Link hover URL disclosure (email content)** - Links in email bodies need status-bar disclosure.

- [ ] **Link context menu (email content)** - Right-clicking a link in an email body should offer actions like Copy Link and related link operations.

- [ ] **Attachment compression: per-mime measurement + report** *(Deferred until real-mailbox data)* - Squeeze runs at the PackStore-write boundary and `log::info!`s `original_bytes -> compressed_bytes` per attachment. No aggregator consumes it. Want a `brokkr` subcommand reporting savings + time per mime, plus an `Unchanged`-rate breakdown to calibrate the passthrough heuristic. Decisions waiting on the data: default `allow_lossy_compression` on/off, skip already-compressed Office docs, move squeeze off the hot path on fast disks, batched fsync vs current per-frame.

- [ ] **`ErofsStore` Linux backend** *(Optional; PackStore stays default)* - Rolling ~256 MB read-only erofs images under `<app_data>/attachment_packs/data-NNNNNN.erofs` with a staging area + bake trigger (shell to `mkfs.erofs`), behind `linux-erofs` cargo feature. Prereqs: extract a `BlobStore` trait (Phase 2 ducked this), parallel `attachment_blobs_erofs` index keyed by `(image_id, path_within_image)`, whole-image eviction only when every blob tombstoned, migration policy (drain PackStore vs only-new-writes). macOS/Windows stay on PackStore.

- [ ] **Gmail `messages.batchGet` attachment batching** *(Deferred; pick up if backfill stalls)* - Phase 7 ships per-attachment `users.messages.attachments.get`. Cheap enough that no measurement justified batching. Revisit if attachment-heavy Gmail backfill of a long retention window becomes a complaint.

- [ ] **Clear-cache button in Storage settings** - `attachment.clear_cache` IPC, `PackStore::tombstone_all_live`, the GC chain, and `GcTrigger::ClearCache` notification all landed. No UI affordance triggers them yet.

- [ ] **"Backfill all attachments for this account" button** - `PrefetchRuntime`'s backfill driver already exists (used for account-add and window-extend). Exposing a one-shot user-triggered backfill in Settings is the only remaining piece.

- [ ] **Attachment chip widget unification** - Reading pane and pop-out viewer have separate attachment-card widgets. Unify them and fold in the future cloud-link chips from `docs/roadmap/cloud-attachments.md`.

- [ ] **Starred thread card background** - The golden tint on starred thread cards uses a fixed `mix()` ratio (`STARRED_BG_ALPHA`) which may not look right across all themes. Needs a GPU-level blend/shader effect that adapts to the theme's background luminance so the starred highlight reads consistently in both light and dark themes.

- [ ] **Star icon: need filled variant** *(Deferred - blocked on sluggrs SVG icon rendering)* - Lucide only has outline icons (confirmed: `star` U+E176, `star-half` U+E20B, no filled variant in the bundled font). Currently uses Unicode `*` as a stopgap, which causes size mismatch and visual jank. Will be resolved by switching to real SVG vector icon rendering (recently implemented in sluggrs, our text renderer) - filled and outline star SVGs can both ship and the toggle just swaps the asset. The button should also not change background color on toggle - just the icon fill.

- [ ] **Autocomplete: cross-field drag-and-drop** - Drag detection works but drop cancels. Context menu "Move to" is the workaround. Needs ghost token rendering and target field hit-testing.

- [ ] **Autocomplete: reuse beyond compose** - Widget only used in compose. Calendar attendee picker and group editor could potentially reuse it.

- [ ] **Contact pills on recipients** - Per `docs/pop-out-windows/problem-statement.md`: recipients in To/Cc fields (in all parts of the app: pop-out view, compose, reading pane thread view, and chat view) should appear as plain text but become contact pills on hover, revealing an inline edit button for quick contact editing. Applies to: reading pane message headers, pop-out message view, compose window recipient display. Currently recipients are plain text everywhere (except pop-out compose window) with no hover interaction. Needs: (1) a contact pill widget that blends with background at rest and reveals pill styling + edit button on hover, (2) display name resolution from the contact system (name → email fallback chain), (3) wiring to the existing `EditContact` flow that opens the settings contact editor. See `docs/pop-out-windows/discrepancies.md` High #4.

- [ ] **Action service: user-facing retry status** *(Deferred - blocked on toast system + missing backend summary helpers)* - The `pending_operations` table backs CRUD + boot recovery via `db::db::pending_ops::*_sync` helpers (enqueue, delete, get, increment_retry, recover_executing, recover_on_boot), and `sync::pending::get_blocked_thread_ids` gates per-thread UI on it. What does NOT exist yet: summary readers like "count by account" or "count failed" - any UI badge would need to add those first. After the readers land, a toast/notification system can surface "N actions pending retry" badges or "Archive failed after 10 retries" persistent notifications. Without this, users have no visibility into silently diverged state.

- [ ] **Action service: native provider batching** *(Deferred - low ROI until bulk ops are common)* - `batch_execute` dispatches per-thread `MailOperation` sequentially within each account. Provider reuse per account already eliminated client construction overhead - remaining cost is network latency (one round-trip per thread). Native batching (Gmail batch API, Graph `/$batch`, JMAP `Email/set`, IMAP multi-UID STORE) would reduce 50 round-trips to 1-3 for bulk operations. `PartialEq` on `MailOperation` enables grouping identical operations; the executor contract already specifies regrouping semantics. Implementation deferred until bulk operations on 50+ threads become a real user workflow.

- [ ] **Raw message source store** - The Source view in the pop-out message viewer currently synthesizes a pseudo-`.eml` from parsed headers + body store content (best effort, not faithful to the original MIME framing). For real on-the-wire raw source we'd need a new `raw_source_store` (zstd-compressed blob store, parallel to `body_store` / inline image store, keyed by `(account_id, message_id)`) populated during sync. Each provider needs a separate fetch path: Gmail `format=raw`, JMAP blob endpoint, Graph `/messages/{id}/$value`, IMAP `BODY[]` (currently parsed-on-the-fly and discarded). Without it, DKIM/ARC verification, the original Received chain, original Content-Transfer-Encoding, MIME boundary strings, header order/casing, and address comments all stay lost - reassembly from the parsed columns can't reproduce any of those byte-exactly. Storage cost is real at the project's "150+ GB cached mailbox" target, so the rollout should consider scope (only newer messages? evict on archive? per-account opt-in?) before turning capture on by default. See `docs/pop-out-windows/discrepancies.md` Medium #7.

- [ ] **Scroll-to-selected in palette** - Arrow keys update `selected_index` but `scrollable::scroll_to` doesn't exist in our iced fork. Needs alternative approach.

- [ ] **`responsive` for adaptive layout** - Collapse panels at narrow window sizes.

- [ ] **Keybinding management UI (Slice 6f)** - Settings panel for viewing, searching, and rebinding shortcuts. Backend ready (override persistence, conflict detection, set/unbind/reset APIs). See `docs/cmdk/app-integration-spec.md` § Slice 6f.

- [ ] **Restore OS-based theme and 1.0 scale** *(Deferred until 1.0)* - Revert to `"System"` theme, persist user prefs.

- [ ] **Bundle SQLite for release builds** *(Deferred until 1.0)* - Re-enable `rusqlite/bundled` feature for release builds so the binary ships a known SQLite version with FTS5 guaranteed. Dev builds use system libsqlite3 for faster compiles.

- [ ] **Reconsider sidebar layout** *(Deferred until right before 1.0)* - Currently the spec says: (1) sidebar should not show any Labels section when "All Accounts" is selected, (2) when a single account is selected, only labels belonging to that account should be shown, and (3) that for providers that have a "folder" concept, the users folders should show in the Labels section. We might need to re-think all 3.

## Roadmap Features - Remaining Work

Features with backend complete but UI or integration work remaining. Each references its roadmap spec.

### Labels Unification - `reference/glossary/folders-labels.md`

Critical: command palette rejects non-Gmail label operations, palette queries use legacy type filtering. Also:

- [ ] **Label picker overlay** - Triggered from reading pane or command palette. Lists all available tag-type labels with colors for apply/remove.

- [ ] **Default colours for `importance:high` / `importance:low` in a user group** - Synth rows have no `server_color_*`, so the first user group that includes one of them needs a colour seed. Decide: pick a sensible default (red/orange shades for `high`, blue/grey for `low`) when the picker adds an `importance:*` row to a fresh group, or surface a colour prompt at add-time.

- [ ] **Resync cadence for Graph `masterCategories`** - Full fetch, no delta endpoint. Today it runs on account add only. Decide a periodic refresh cadence so user-added/renamed Outlook categories appear without an app restart.

- [ ] **Stable smart-folder group binding** - The landed `label:` SQL resolves by group name at execution time, so a group rename silently changes which group a persisted smart-folder query resolves to. Binding by `group_id` would survive renames, but requires changing the persisted smart-folder representation away from plain text.

### Search - `docs/search/problem-statement.md`

Backend pipeline exists (parser, SQL builder, Tantivy, unified router). **29 discrepancies remain** - see `docs/search/discrepancies.md`. Critical: combined path applies free text in SQL before Tantivy ranking, Tantivy-only results show wrong message metadata, date boundaries inconsistent across engines. Also typeahead, pinned search lifecycle, and smart folder management gaps.

- [ ] **Promote pinned search to Smart Folder** - Sidebar pinned searches need an action that converts a pinned search into a Smart Folder.

### Calendar - `docs/calendar/problem-statement.md`

Views, editor, pop-out, sidebar all partially implemented. See `docs/calendar/discrepancies.md` for the live list. Backend now covers TZID/VTIMEZONE resolution (CalDAV) and Windows timezone names (Graph), CalDAV is consolidated on `rtsk::caldav` (calcard parser, ctag/etag incremental sync), `canEdit` flows from Graph/Google access roles to a `calendars.can_edit` column, and meeting-invite detection populates `messages.has_meeting_invite` / `meeting_invite_method` at insert time. RRULE expansion now handles BYDAY/BYMONTHDAY/BYMONTH on top of the FREQ/INTERVAL/COUNT/UNTIL baseline. Still open: drag interactions, RSVP actions, runtime reminder timer, meeting-invite UI affordances, permission gating on action buttons.

**Calendar UI issues (observed 2026-04-04):**

Event popover (quick-glance card):
- [ ] Position is wrong - currently right-aligned in the calendar view, should anchor near the clicked event pill
- [ ] Styling needs work (visual polish pass)
- [ ] Clicking a different event pill while the popover is open just closes the popover instead of closing and immediately opening the new event's popover. Root cause: `popover_stack` (`crates/app/src/ui/calendar.rs`) renders a full-viewport `mouse_area` backdrop with `on_press(ClosePopover)` on top of the calendar base, which swallows the click before it reaches the underlying event pill. Will be resolved by the deferred AnchoredOverlay migration (see "Calendar event detail popover -> AnchoredOverlay" above) - anchoring the popover near the pill removes the need for a click-blocking backdrop.

Event detail modal:
- [ ] Needs significant visual and layout work

Event editor modal:
- [ ] Does not adhere to the editor spec at all - needs a full implementation pass
- [ ] Discarding changes doesn't work (but doesn't save changes either, so no data loss)

Month view:
- [ ] Event pill overflow still not filling actual available space - current fix uses CALENDAR_CELL_MIN_HEIGHT, so cells only pack events to the minimum height; when the window is taller, cells grow but still cap at the same event count. Needs a layout-aware widget that measures actual rendered cell height.

Week view:
- [ ] All-day events are not laid out properly at the top of the day columns

### Generic OAuth - `docs/generic-oauth/problem-statement.md`

Core OIDC discovery + OAUTHBEARER + WebFinger + custom-scopes + dynamic-registration + Custom OIDC wizard providers + IT-distributable config file implemented. **1 discrepancy remains, 1 in flight** - see `docs/generic-oauth/discrepancies.md` (audit refreshed 2026-05-19). Remaining: SMTP path is XOAUTH2-only (#5 blocked on lettre not exposing OAUTHBEARER). In flight: no IMAP SASL auto-detect from CAPABILITY (#6 blocked on an async-imap fork that exposes pre-auth `Client::capabilities()`; plan at `.plans/moonlit-herding-cookie.md` § Slice A). Discovery cascade has four passing Lua scripts (happy path, bare-domain fallback, autoconfig upgrade, negatives) covering items 1-6 in Test Harness § OIDC discovery harness.

### Chats - `docs/chats/problem-statement.md`

Backend plumbing complete (schema, sync, core APIs, timeline view). Feature unreachable by users. **7 discrepancies remain** - see `docs/chats/discrepancies.md`. Critical: no sidebar entry point, no body text rendering, no mark-read, no inline compose.

- [ ] **Per-bubble user-account indicator** - Spec (`docs/chats/problem-statement.md` § "What about multi-account contacts?", L201-205) calls for "a subtle account indicator (the account's color dot or abbreviation)" on each chat bubble so the user can tell which of *their own* accounts a given message belongs to when a contact spans multiple accounts (e.g. work + personal). Currently unimplemented - bubbles render with no account marker. Likely a small colored dot using `account.account_color` near the bubble corner, or a short abbreviation tag - low-visual-weight, since most chats are single-account in practice.

- [ ] **Conversation party name/identity in chat view** - The spec is silent on showing the contact's name *within* the chat view itself; the only on-screen identity cue today is the sidebar pill (which can scroll out of frame). This is a spec gap, not a deferred feature. We probably want a slim header bar above the timeline with the contact's name + avatar (and email under it) so the active chat is identifiable at-a-glance. Resolve the spec gap before implementing - decide whether it's a sticky header, a bubble-level sender label, or a toolbar-style row, then update `docs/chats/problem-statement.md` § "A view mode, not a message type".

### Tracking Blocking - `docs/roadmap/tracking-blocking.md`

Sanitization pipeline, MDN detection, tracking pixel detection, URL cleaning all done. Remaining:

- [ ] **Read receipt prompt UI** - `read_receipt_policy` table and `mdn.rs` policy resolution exist. Need UI prompt when opening a message with `mdn_requested=true`: "Send read receipt?" with per-sender/per-account policy options (ask/always/never).
- [ ] **Read receipt policy management in Settings** - Settings panel for configuring default MDN policy per account and per-sender overrides.

### Cloud Attachments - `docs/roadmap/cloud-attachments.md`

The hand-rolled OneDrive (`gmail/src/gdrive.rs`) and Google Drive (`graph/src/onedrive.rs`) upload code has been deleted (bifrost-migration B9) - it had no production caller. Outgoing hosting now goes through bifrost's capability-gated `Account::host_attachment` (Google -> Drive, Graph -> OneDrive), reached via `AttachmentByteSource::host_large_attachment` in `crates/service/src/bifrost/attachment.rs`, which has no caller yet. `core/cloud_attachments.rs` now holds only the pure incoming-link detectors (`detect_cloud_links`, `extract_gdrive_file_id`, `CloudProvider`) - `enrich_onedrive_link`/`enrich_gdrive_link` and their `GraphClient`/`CloudMetadata` plumbing had no caller either and were deleted at bifrost-migration B15 along with the rest of `core`'s provider-crate deps. Remaining:

- [ ] **Compose UI for cloud attachment flow** - Size threshold detection in compose, prompt to upload to cloud, upload progress indicator, insert link into message body, wired against `AttachmentByteSource::host_large_attachment`.
- [ ] **Offline upload queue** - Queue uploads when offline, retry when connectivity returns.

### Public Folders - `docs/roadmap/public-folders.md`

EWS SOAP client, autodiscover routing, offline sync, IMAP NAMESPACE public folders, DB schema all done. Sidebar pins done (2026-03-22). Remaining:

- [ ] **Public folder browser** - Lazy-load tree widget for browsing the hierarchy and pinning folders. Uses existing `browse_public_folders()` API.
- [ ] **Reply/post wiring** - Connect compose to `CreateItem` EWS operation for replies and posts to public folders.

### Shared Mailboxes - `docs/roadmap/shared-mailboxes.md`

Exchange Graph sync + Autodiscover + sidebar integration done. Remaining:

- [ ] **Gmail delegation support** - Blocked (API limitation). Send-As aliases work.
- [ ] **Per-mailbox sync depth config** - Currently hardcoded to 30 days. No per-mailbox setting.

### JMAP Sharing - `docs/roadmap/jmap-sharing.md`

All 6 backend phases complete (discovery, sync, rights, subscription, notifications, identity resolution). Remaining app-crate UI integration:

- [ ] **Subscription toggle in sidebar** - `NavigationFolder.is_subscribed` is populated from JMAP `isSubscribed`. App needs a UI toggle (context menu or button) on shared account labels that calls `JmapOps::subscribe_mailbox()` / `unsubscribe_mailbox()`. These accept an optional `jmap_account_id` for shared accounts.

### Labels - `reference/glossary/folders-labels.md`

- [ ] **Label picker UI** - Overlay for applying/removing tag-type labels from messages. Triggered from reading pane or command palette. Lists all available labels with colors. Provider dispatch via `add_tag()`/`remove_tag()`.

### Mentions - `docs/roadmap/mentions.md`

- [ ] **Compose @-autocomplete** - Detect `@` in compose editor, show floating contact picker, insert `@Display Name` text, auto-add to To/CC if not already a recipient. Works identically across all providers (cosmetic markup only).

### Scheduled Send - `docs/roadmap/scheduled-send.md`

Backend complete (server delegation + overdue handling). Missing UI.

- [ ] **Schedule picker UI** - Date/time picker in compose toolbar. Delegates to Exchange (deferred delivery) or JMAP (FUTURERELEASE) server-side, falls back to local timer for Gmail/IMAP.
- [ ] **"Scheduled" virtual folder** - Virtual folder view showing all pending scheduled messages across accounts with edit/reschedule/cancel.

### Signatures - `docs/roadmap/signatures.md`

Backend complete (Gmail + JMAP sync). Exchange fetch permanently blocked (no public API, Microsoft confirmed no plans).

### Send-As Aliases

Backend reads + default-alias selection work (`db_get_aliases_for_account`, `db_get_default_alias`, `db_set_default_alias`; provider sync populates `send_as_aliases` on Gmail). No roadmap doc yet.

- [ ] **Manual alias edit UI** - Settings surface for creating / editing / deleting send-as aliases independent of provider sync (display name, reply-to, signature binding, treat-as-alias toggle, verification status). The speculative `db_upsert_alias` helper (10 positional args) was deleted during a dead-code triage; the eventual UI work should write a focused `(WriterPool, SendAsAlias)` upsert with a params struct rather than reviving the old shape. Schema is in `crates/db/src/db/schema/04_compose.sql` (`send_as_aliases`); the `(account_id, email)` uniqueness constraint is the conflict target.

### Auto-Responses - `docs/auto-responses/problem-statement.md`

The hand-rolled per-provider `fetch_*`/`push_*` read/write functions in
`crates/core/src/auto_responses.rs` had no caller and were deleted in B13.
Server-side vacation settings are now reachable only as a capability-dispatched
pin (`vacation_get`/`vacation_set`) behind `AccountSettingsSurface`
(`crates/service/src/bifrost/settings.rs`), with no UI caller yet. The local
status-bar read (`any_auto_response_active`) is unaffected. Remaining:

- [ ] **Auto-reply settings UI** - Per-account editor in settings. Toggle, date pickers, message editor, audience selector. Internal/external tabs for Exchange only. Provider HTML must be sanitized before rendering (stored unsanitized in DB).

### IMAP CONDSTORE/QRESYNC - `docs/roadmap/imap-condstore-qresync.md`

Phases 1-2 complete. Phase 3 blocked on upstream.

- [ ] **QRESYNC VANISHED parsing** - Blocked on `async-imap` upstream (Issue #130). UID-based deletion detection works as workaround.

## Blocked / External

- [ ] **Ship a default Microsoft OAuth client ID** - Manual Azure AD registration task.
- [ ] **QRESYNC VANISHED parsing** - Blocked on `async-imap` upstream (Issue #130). See above.

## Remaining Enhancements (HTML rendering)

The DOM-to-widget pipeline (`html_render.rs`) handles structural HTML but has significant fidelity gaps. Remaining:
- [ ] Remote image loading with user consent (`block_remote_images` setting exists but disconnected from `render_html` - function signature needs context parameter)
- [ ] Table rendering (table-for-layout is the hardest - no `<table>`/`<tr>`/`<td>` handling at all)
- [ ] Image caching (`HashMap<String, image::Handle>`) - no `iced::widget::image` usage in app crate

## Security / Bug Findings (unfixed)

- [ ] **CalDAV password still stored plaintext** *(narrowed from the broader 2026-05-01 codex finding; mainline mailbox-credential paths were closed by Phase 6a)* - The mainline OAuth-token / IMAP-password / SMTP-password paths now encrypt at the Service handler boundary: `account.create` and `account.update_tokens` route `AccountCredentials::Plaintext` through `encrypt_optional_credentials` (`crates/service/src/handlers/account.rs:32-61, 247-289`), and the read side fail-closes via `StoredSecret::parse` (`crates/common/src/crypto.rs:144-168`) - the tolerant `decrypt_or_raw` / `decrypt_if_needed` helpers are gone. CalDAV is the holdout: `account.update` accepts `caldav_password: Option<String>` and writes it verbatim with no encryption call (`crates/service/src/handlers/account.rs:96-111` documented at `crates/service-api/src/account.rs:19-21`), and the CalDAV reader still uses the masking pattern `if is_encrypted { decrypt } else { raw }` (`crates/calendar/src/caldav/mod.rs:271-275`) so plaintext rows continue to "work." Fix shape: extend the credential-envelope pattern to caldav_password (encrypt at handler), then delete the `is_encrypted ? decrypt : raw` fallback in the reader. Regression test gate (still missing for any field): assert that the value stored in `accounts.imap_password|smtp_password|access_token|refresh_token|caldav_password` after a write IPC is never `==` to the plaintext input.

- [ ] **Mail content stores not encrypted at rest** *(verified 2026-05-19; citations refreshed - the underlying claim is unchanged but the attachment storage layer was rewritten under it)* - Bodies are written compressed-but-unencrypted via `BodyStoreWriteState::put` / `put_batch` (`crates/stores/src/body_store.rs:164, 198`; the cited line 117 is now in the zlib `decompress` helper - compression is not a security boundary). Inline images are still raw SQLite blobs through `InlineImageStoreWriteState::put` / `put_batch` (`crates/stores/src/inline_image_store.rs:111, 140`). The Phase 3 "PackStore wired, flat cache retired" commit (`e6bb227a`) replaced `attachment_cache.rs` with the appending pack format in `crates/stores/src/attachment_pack.rs` - it still writes raw bytes (`attachment_pack.rs:380-382`: `writer.file.write_all(&header); writer.file.write_all(&bytes); writer.file.sync_all();`), so the migration moved the bytes but did not add encryption. Zero matches for `encrypt|cipher|AES|GCM` across `body_store.rs`, `inline_image_store.rs`, or `attachment_pack.rs`. Fix: envelope-encrypt with AES-256-GCM using the same `BootSharedState` key the credential path uses, or document explicitly that content at rest relies on OS / full-disk encryption.

## rte Bug Findings (2026-07-28 full-crate bug hunt, unfixed)

Whole-crate read of `crates/rte` (all 15 files) plus source verification against
the sluggrs iced fork. Items 1-5 were each confirmed by a failing repro test
before fixing; the fixes landed 2026-07-28 with in-module regression tests
(rules.rs, editor_state.rs, document.rs, html_serialize.rs, html_parse tests).
Item 6 remains open.

- [x] **1. Typing with a pending style leaves the caret BEFORE the typed character** - Fixed: `update_cursor_after_ops` (`crates/rte/src/widget/editor_state.rs`) now scans ops backwards for the last cursor-determining op (InsertText / DeleteRange / SplitBlock / MergeBlocks) instead of only inspecting the last op, so trailing fixup ops (`ToggleInlineStyle` after an insert, `SetBlockType` after a heading-end split) no longer mask the caret update. This also fixed a sibling latent bug: Enter at the end of a heading used to leave the caret at the heading's end instead of the new paragraph. Pinned by `pending_style_insert_places_caret_after_text` and `enter_at_end_of_heading_moves_caret_to_new_paragraph`.

- [x] **2. Replacing a styled selection with typed text drops the styling** - Fixed: `resolve_insert` (`crates/rte/src/rules.rs`) now applies the selection-delete ops to a scratch clone and predicts the inherited run style against that post-delete document, while the *desired* style comes from the new `style_at_position` (right affinity - the first selected character, i.e. what the replaced text looked like). Pinned by `insert_replacing_styled_selection_keeps_style`, `insert_replacing_mixed_selection_takes_first_char_style`, and the editor-level `replacing_styled_selection_with_typed_text_keeps_style`.

- [x] **3. Deleting from just after an image into the next block destroys the image, and undo cannot restore it** - Fixed at the rules layer: the new `post_delete_position` (`crates/rte/src/rules.rs`, `pub(crate)`) normalizes a selection start sitting at the trailing edge of an atomic block (`(atom, 1)` → `(atom + 1, 0)`) before the `DeleteRange` op is built, so the atom survives and the op/undo record are self-consistent. Used by `build_delete_selection`, `resolve_insert`, `resolve_split_block`, and `EditorState::paste_slice`. Note the raw `EditOp::DeleteRange` path in operations.rs still has the destructive shape if handed an un-normalized range directly - all rule-resolved paths normalize now. Pinned by `delete_from_atom_trailing_edge_spares_atom`, `delete_from_atom_trailing_edge_cross_block_undo_restores`, and the editor-level pair `delete_selection_starting_after_image_{keeps_image,undo_restores}`.

- [x] **4. `<br>` parses to a literal `"\n"` run but is never serialized back as `<br>`** - Fixed: `serialize_run` (`crates/rte/src/html_serialize.rs`) splits run text on `'\n'` and emits `<br>` between the escaped segments, so soft line breaks survive the round trip (signatures were the main victim). Side benefit: `<pre>` content with embedded newlines also serializes without raw newlines now. Pinned by `newline_in_run_serializes_as_br`, `newline_inside_styled_run_serializes_as_br`, `round_trip_br`, and `round_trip_br_inside_styled_signature_line`.

- [x] **5. `Document::slice` demotes fully-covered start/end blocks to `Paragraph`** - Fixed at the slice layer: `block_with_runs_like` is now a shared `pub(crate)` helper in `document.rs` (the private copies in operations.rs and rules.rs were consolidated onto it), and `Document::slice` uses it for the start/end blocks in both the multi-block and partial-single-block paths, so headings and list items keep their type (and `ordered`/`indent_level`) in the slice. Pinned by `slice_multi_block_preserves_edge_block_types` and `slice_partial_single_block_preserves_block_type`. Deliberately NOT changed: the paste side still merges first/last slice-block runs into the surrounding blocks regardless of `open_start`/`open_end` - that edge-merging is pinned as intended behavior by `copy_paste_cross_block` and `copy_paste_preserves_multi_block_structure_at_cursor_mid_block`, so a fully-copied heading pasted mid-paragraph still lands as paragraph text. If block-level paste for closed slices is ever wanted, it is a deliberate UX change to those two tests, not a bug fix.

- [ ] **6. Mouse hit-testing returns BYTE offsets, but `DocPosition` is grapheme-based - clicks misplace the caret in any non-ASCII text** - The fork's `Paragraph::hit_test` returns `Hit::CharOffset(cursor.index)` where `cursor.index` is cosmic-text's *byte* index within the buffer line (`sluggrs/repos/iced/graphics/src/text/paragraph.rs:299-306`), while `grapheme_position` explicitly takes a grapheme index (same file, `:390-`, comment "index represents a grapheme"). `hit_test_content_point` (`crates/rte/src/widget/mod.rs`) feeds the byte offset straight into `DocPosition`, and `build_line_starts` stores byte offsets that `find_line_for_offset` then compares against grapheme offsets. Every click in text containing æ/ø/å/emoji lands the caret progressively further right of the click point (each non-ASCII char is 2+ bytes but 1 grapheme), and subsequent edits go to the wrong offset. For a Norwegian-language mailbox this bites on nearly every line. Secondary wrinkle: `hit_test` ignores `cursor.line`, so in a paragraph containing `"\n"` runs (from `<br>`) the returned index is relative to the buffer line, not the block. Fix: convert byte→grapheme at the hit-test boundary (block text is available at both call sites), and do the same for `build_line_starts` entries.

- [x] **7. Ctrl+Shift+Z redo is dead; Ctrl+B/I/U/C/V/X/Z/A/Y fail with Shift or CapsLock** - Fixed pragmatically: `map_command_shortcut` (`crates/rte/src/widget/input.rs`) lowercases the logical-key string before matching, so the uppercased keys that Shift/CapsLock produce ("Z" for Ctrl+Shift+Z) hit the match arms. Pinned by `ctrl_shift_z_uppercase_logical_key_redoes` and `ctrl_b_uppercase_logical_key_toggles_bold`. Remaining (deliberate): non-Latin keyboard layouts would need `Key::to_latin(physical_key)` (as the fork's own widgets use - `sluggrs/repos/iced/widget/src/text_editor.rs:1149-1156`), which requires threading `physical_key` through `map_key_event` and its ~40 test call sites.

Smaller confirmed issues from the same hunt, roughly in severity order:

- [x] **rte: drag-selecting above the viewport jumps the selection to the document top** - Fixed: the upward auto-scroll branch of `handle_drag` (`crates/rte/src/widget/mod.rs`) now hit-tests at the freshly-updated `scroll_offset` (viewport top in content coordinates), mirroring the below-viewport branch.

- [x] **rte: Ctrl+X on a selection whose plain text is empty does nothing** - Fixed: `handle_keyboard` (`crates/rte/src/widget/mod.rs`) now publishes `Action::Cut` unconditionally (EditorState no-ops on a collapsed selection); only the clipboard write remains gated on non-empty text. Cutting an image with empty alt text now captures and deletes it.

- [ ] **rte: single click on a link always fires `LinkClicked`, never places the caret** - `handle_mouse` (`crates/rte/src/widget/mod.rs:471-483`) makes link text uneditable by mouse - the only way into it is arrowing. If deliberate, fine, but Ctrl+click-to-follow / plain-click-to-edit is the convention in editable surfaces.

- [ ] **rte: `find_signature_end` assumes an attribution paragraph sits immediately before the blockquote** - (`crates/rte/src/compose.rs:207-215`) returns `blockquote_index - 1` unconditionally. If the user deleted the attribution line, or the signature HTML itself contains a `<blockquote>`, `replace_signature` removes the wrong range and can leave a stale signature line behind on From-account switch.

- [ ] **rte: cross-block delete `PosMap`s under-describe the change** - `apply_cross_block_delete` (`crates/rte/src/operations.rs:683-687`) records `PosMapEntry { old_len: 0, .. }`, so positions inside the deleted tail of the start block are not collapsed to the deletion point (the `CrossBlockDelete` structural arm only collapses positions in *later* blocks). Currently masked because `update_cursor_after_ops` overwrites the selection and `UndoStack::map_cursors` has no callers, but it is a trap for any future `PosMap` consumer.


## Remaining Enhancements (other)

- [ ] **iced_drop for cross-container DnD** - Custom DragState works for list reorder. iced_drop needed for: compose token DnD, label drag-to-file, calendar event dragging, attachment drag zones (the compose-window two-zone overlay - see `docs/pop-out-windows/discrepancies.md` High #1).
- [ ] **Read receipts (outgoing)** - MDN support. See `docs/roadmap/tracking-blocking.md`.
- [ ] **Inline image store eviction UI** - Settings control for store size (128 MB hardcoded).

- [ ] **Provider push notifications (remaining)** - JMAP WebSocket push is wired. Still missing: IMAP IDLE (persistent connection per folder), Graph/Gmail (poll-based, needs tuning - true push requires cloud infrastructure).
- [ ] **Pop-out Print** - OS print dialog integration for message view and compose pop-out windows. Platform-specific, no iced precedent. Needs investigation. See `docs/pop-out-windows/discrepancies.md` Medium #9 (and High #3 for the missing compose-header Print button).
- [ ] **Modal dialog content unification (GNOME HIG / libadwaita)** - The `alert_dialog` / `form_dialog` primitives in `ui/dialog.rs` now lock down GNOME HIG / `AdwAlertDialog` semantics (window-like card via `ContainerClass::DialogCard`, `TEXT_HEADING` title, `TEXT_MD` secondary body, right-aligned button row, libadwaita action appearances via `ButtonClass::Suggested` / `ButtonClass::Destructive`). Migrated: compose discard / link / save-as-group, calendar delete-event / discard-changes. Remaining work:
  - **Add-account modal** (`main.rs::view_with_add_account_modal`) is a multi-step flow, not a simple alert - keep its own card but reuse `ContainerClass::DialogCard` and the action-row layout pattern.
  - **First-launch onboarding** (`main.rs::view_first_launch_modal`) is a full-screen surface, not a stacked modal; leave as-is.
  - **Inline confirmation rows** in settings (delete-account in `accounts.rs`, delete-signature in `signatures.rs`, delete-group in `groups.rs`, delete-contact in `contacts.rs`) live inside the settings *Sheet*, not a Modal stack. Different pattern; out of scope for `alert_dialog`. Should still get a unified inline-confirm helper, but distinct from the dialog primitive.

- [ ] **Rich text editor (rte) post-review gaps** - Surfaced during the 12-finding correctness review. None are regressions; all are interactions between the recent fixes and the existing flat `DocPosition` model.
  - `is_atomic_block()` is defined as `!is_inline_block()`, so it includes `BlockQuote` alongside `Image` and `HorizontalRule`. Backspace at the start of a paragraph immediately following a `BlockQuote` now removes the entire quoted reply (not a no-op, not a merge). Acceptable but aggressive in the compose pop-out where BlockQuotes hold reply content - if user feedback bites, split atomic-vs-container behaviour in `resolve_delete_backward` / `resolve_delete_forward` (`crates/rte/src/rules.rs`).
  - `link_at_content_point` (`crates/rte/src/widget/mod.rs`) returns `None` when `entry.paragraph()` is `None`, which is the case for container blocks (`BlockQuote`, list groups). Single-clicking a link inside a quoted reply still falls through to caret placement instead of emitting `Action::LinkClicked`. Matches the existing "container content isn't `DocPosition`-addressable" limitation - revisit when/if container content becomes addressable.
  - Caret rendering inside an atomic block: `draw_cursor` (`crates/rte/src/widget/mod.rs`) falls into the no-paragraph branch and draws at `para_origin_x` for both offset 0 and offset 1, so arrowing across an `Image` or `HorizontalRule` produces no visible cursor movement even though the offset advances. Functionally fine (Backspace/Delete on the post-atom offset still removes the atom); purely a visual fidelity gap.
  - `paste_plain_text` (`crates/rte/src/widget/editor_state.rs`) splits on `\n` after CRLF normalization, so a trailing newline (e.g. `"alpha\n"`) produces an extra empty paragraph at the end. Likely intended (preserves explicit blank-line intent), but worth confirming against real-world paste sources before treating as final.

- [ ] **`html_render` post-review gaps** *(Bridge fixes only - litehtml-rs at `/home/folk/Programs/litehtml-rs` is the eventual replacement)* - Surfaced during the 11-finding review of `crates/app/src/ui/html_render.rs`. None are regressions; each is a known limitation of the targeted fixes that landed for the bridge period.
  - **Inline image frame width.** `render_cid_image` uses `width(Length::Fill) + ContentFit::ScaleDown`. Large images correctly scale down to body width, but small images now reserve the full body width with empty space around the rendered pixels. iced's `image` widget doesn't expose `max_width`; a real "shrink to natural, cap at container width" needs a `responsive` wrapper or a natural-dimension query that picks `Length::Fixed(min(natural_w, available_w))`. Verify visually before treating as final.
  - **Heading style fidelity.** `Block::Heading(String, u8)` only stores plain text, so `<h1>Hello <em>world</em></h1>` collapses to "Hello world" rendered semibold - the `<em>` italic run is lost. Promoting to `Block::Heading(Vec<InlineSpan>, u8)` would restore fidelity but ripples through all heading rendering call sites; not worth doing now if litehtml-rs is close.
  - **Inline styles inside `<pre>`.** Style flag bumps are gated by `!self.in_pre` so `<pre>plain<b>bold</b>plain</pre>` flattens to `Preformatted("plainboldplain")` - bold is lost. Correct semantics for pre-as-plain-text but wrong for source-with-syntax-highlighting. Same path-of-least-resistance trade-off until litehtml-rs.
  - **Trailing-text-after-nested-list ordinal renumbering.** `<ol><li>outer<ul>...</ul>after</li></ol>` parses as `1. outer / • inner / 2. after`. The "2." is a side-effect of the flat block model emitting the trailing inline content as its own outer-list item. Same flat-model compromise as the rte parser - users may or may not notice.

## Test Harness

Architecture and design rationale stay in `reference/glossary/harness.md`. The milestone roadmap is retired - remaining work is captured here.

### Tests unlocked by saehrimnir 45bf850..28017e7

These depend on installing a saehrimnir binary at or after `28017e7`
and mirroring any needed upstream fixtures into
`crates/app/tests/sync-fixtures/`.

- [ ] **Graph shared-mailbox `/users/{id}` mail sync** - Drive
  Graph sync against a secondary account in `multi-account-small` through
  bifrost's namespaced-container path (the legacy `GraphClient::
  for_shared_mailbox` this item was written against was deleted at
  bifrost-migration B15; shared-mailbox Graph sync has been container-owned
  since B12). Assert shared mailbox folders, messages, attachment metadata,
  and delta cursors are stored under the shared namespace instead of the
  personal account scope.
- [ ] **Graph `/users/{id}` calendar scoping** - Extend the
  shared-mailbox sync harness to cover Graph calendar reads through
  `/v1.0/users/{id}/...`. Assert per-account calendars, events, and
  delta links stay isolated.
- [x] **Graph master category label sync** - Landed.
  `graph-categories-small.toml` exercises the resident Graph auxiliary pass
  through the `graph-master-category-label-sync` script. Sync runs
  via the new initial-sync invocation; `cat:<displayName>` rows land
  with `label_kind = 'tag'`, the correct `account_id`, sort order
  matching the fixture index, and Exchange preset colours mapped
  through `label-colors::preset_colors` (preset0/2/15 verified, no
  preset → color_bg/fg null).
- [ ] **Graph category shared-mailbox path hardening** - Combine
  master categories with a multi-account fixture. The category-definitions
  read (`SyncEngine::category_definitions_list`, `service/src/bifrost/aux/
  graph.rs`) is per-account-scoped through bifrost-graph's own account
  routing as of bifrost-migration B15 (the legacy `graph_label_sync()` /
  `GraphClient::api_path_prefix()` hardcoded-`/me` path this item was
  written against no longer exists); assert category labels from one
  mailbox never appear in another mailbox's label set or sidebar scope.
- [x] **Google OAuth token account binding: Gmail** - Landed via
  `gmail-oauth-multi-account`. Two minted tokens against
  `multi-account-small` give each Gmail account its own messages
  and labels; cross-account leakage is asserted on the
  `labels.account_id` column rather than ID equality since system
  labels (DRAFT, INBOX, ...) intentionally repeat per principal.
- [x] **Gmail SendAs signature import** - Landed via
  `gmail-send-as-multi-account-import`. `multi-account-small` now
  carries `[[send_as]]` rows per account; the test asserts the
  Gmail-imported signatures table (server_id IS NOT NULL) holds the
  expected HTML body, display-name-decorated `name`, `is_default`
  flag, and source `gmail_sync`. Local "Harness" signatures inserted
  by TestSeedAccount are filtered out.
- [ ] **Gmail SendAs fault injection** - Use Lua
  `on("gmail", "send_as", fn)` to force list/get/patch failures.
  Assert signature import reports a provider failure without corrupting
  local signatures, and writeback leaves the expected pending or retry
  state.
- [ ] **Gmail SendAs token account binding** - Extend the Gmail
  multi-account OAuth tests to cover SendAs identities. Mint tokens for
  primary and secondary accounts, sync signatures for both accounts,
  and assert each account only imports or patches its token-bound
  identity.
- [x] **Google OAuth token account binding: Calendar and People** -
  Landed via `google-oauth-multi-account-calendar-people`.
  `multi-account-small` now carries per-account `[[calendar]]`,
  `[[event]]`, `[[contact_folder]]`, and `[[contact]]` rows; with
  one minted token per account the harness runs both
  `start_sync` (Gmail+People) and `start_calendar_sync` and
  asserts each principal's Google calendar, event, and contact
  rows are scoped to its own `account_id`. Missing-token fallback
  is not exercised yet.
- [x] **CalDAV multi-account principal scoping** - Landed via
  `caldav-multi-account-principal-scoping` against the new
  `multi-account-calendar-small.toml` fixture. Saehrimnir gained
  Basic-Auth username resolution on the bootstrap PROPFIND so each
  principal sees its own `/principals/{user}/` URL instead of the
  primary's; the harness asserts per-principal calendars and events
  only.
- [x] **CalDAV secondary-principal write isolation** - Landed via
  the new service-harness script
  `m6/calendar_actions_caldav_multi_account`. Create / Update /
  Delete through the secondary principal land exclusively under
  `/calendars/account-secondary/...`; the request log shows zero
  PUT/DELETE traffic against the primary's home and the primary
  DB never picks the mutations up after its own sync.
- [ ] **CalDAV `MKCALENDAR` create-calendar action** - Once the
  Ratatoskr create-calendar path is exposed in the harness, create a
  calendar against a CalDAV account and assert saehrimnir records
  `MKCALENDAR`, preserves display name / calendar color, and the next
  sync imports the new calendar. Include duplicate-id and unknown
  principal failure cases.
- [ ] **Cross-protocol calendar creation visibility** - After a
  CalDAV `MKCALENDAR`, sync JMAP Calendar and Graph Calendar against
  the same mock fixture. Assert JMAP `Calendar/changes` reports the
  created calendar and Graph `/me/calendars` lists it, proving
  saehrimnir's shared `calendar_created` transition is visible across
  protocol surfaces.
- [x] **IMAP LOGIN multi-account binding** - Landed via
  `imap-login-multi-account` against `multi-account-small`. Each
  ratatoskr account's email matches a fixture principal so
  saehrimnir's `account_id_for_username` routes the connection;
  the harness asserts disjoint per-account inboxes and at least one
  LOGIN/LIST/SELECT per account in the mock request log.
- [ ] **IMAP XOAUTH2 / OAUTHBEARER account binding** - Deferred.
  async_imap hangs against saehrimnir's two-round-trip XOAUTH2 /
  OAUTHBEARER continuation flow: the saehrimnir test for inline
  SASL-IR works, but async_imap sends `AUTHENTICATE XOAUTH2`
  without a SASL-IR token and never recovers from the `+`
  continuation prompt. Needs deeper saehrimnir-side debugging
  before this test can land.
- [x] **SMTP AUTH account attribution** - Landed via
  `t1/smtp_auth_multi_account_attribution`. Two accounts seeded
  from `multi-account-small` send through ActionSend; the
  `/test/smtp/submissions` log records the right `account_id`
  (resolved by saehrimnir's `account_id_for_username` from SMTP
  AUTH PLAIN), the `auth_mechanism = "PLAIN"`, the `from`, and
  the recipient list for each submission.
- [ ] **SMTP AUTH failure callback** - Use Lua
  `on("smtp", "AUTH", fn)` to force an auth failure. Assert the send
  action reports the right provider failure, does not record a
  successful submission, and leaves any retry / pending-op state in
  the expected shape.
- [ ] **Expand recurrence read matrix** - Daily, yearly+BYMONTH,
  and a wider row-field assertion (description, location,
  organizer_email) now ride the existing
  `*-calendar-recurrence-initial` scripts. Still missing: EXDATE
  round-trip (no harness-visible occurrence/exception column today
  - fixture's `recurrence_exdates` is parsed but not surfaced) and
  timezone handling (saehrimnir emits everything as UTC). Both
  require a harness extension before they're testable.
- [ ] **Recurrence write matrix** - Create and update recurring
  events through the Service calendar action path for Graph, Google
  Calendar, JMAP Calendar, and CalDAV. Assert the request log carries
  provider-native recurrence payloads and a follow-up sync imports
  the same recurrence metadata back into local state.
- [ ] **Cross-protocol recurring-event mutation deltas** - Mutate a
  recurring event through one mock protocol, then sync another
  protocol backed by the same fixture state. Cover at least Graph
  after CalDAV, Google Calendar after Graph, and JMAP Calendar after
  Google Calendar so the shared change-log recurrence path is pinned.

### OIDC discovery harness

The entire discovery cascade (`crates/core/src/discovery/`) has no Lua harness coverage today. WebFinger (`webfinger.rs`, shipped 2026-05-19), OIDC discovery (`oidc.rs`), Mozilla autoconfig (`autoconfig.rs`), MX lookup (`mx.rs`), JMAP `.well-known/jmap`, and port probing all run only in unit tests. Existing OAuth harness tests (`jmap-oauth-recovery.lua` etc.) sidestep discovery entirely by pre-seeding accounts with all OAuth fields resolved.

Saehrimnir discovery surface shipped 2026-05-19 and is installed locally - see `../sæhrimnir/notes/ratatoskr-discovery-surface.md`. Three routes mount on the JMAP HTTP listener: `GET /{prefix}/.well-known/webfinger`, `GET /{prefix}/.well-known/openid-configuration`, `GET /{prefix}/mail/config-v1.1.xml`. Fixtures use `[discovery."<prefix>".{webfinger,oidc,autoconfig}]` tables. Prefix is opaque - the chained-issuer document for our WebFinger negative tests lives at e.g. `idp/realms/corp`, distinct from the email domain's prefix `corp.test`. URLs in fixtures are either absolute (literal) or path-relative starting with `/` (emit-time prefixed with the live listener base URL); `${BASE}` substring substitution applies *only* inside `autoconfig.raw_body`. Negative tests get `raw_body` + `raw_content_type` escape hatches on each route, and the loader doesn't enforce OIDC issuer self-claim so a fixture can stage a mismatch and assert ratatoskr rejects it.

Ratatoskr-side work:

- [x] **Discovery probes route through saehrimnir in test mode** - `discovery_client()` + `rewrite_for_test_harness()` helpers in `crates/core/src/discovery/mod.rs` rewrite `https://{host}/...` to `${BASE}/{host}/...` and relax `https_only` when `RATATOSKR_TEST_JMAP_ENDPOINT` is set (reuses the existing JMAP env-var slot since saehrimnir mounts discovery on the JMAP listener; no brokkr-side env-var schema change needed). `is_valid_https_url` loosened to accept `http://` URLs whose origin matches the configured test base, so chained-issuer hrefs survive validation. Used by `webfinger::probe`, `oidc::probe_issuer`, and `dyn_registration::register`. Production paths untouched - the env var is never set there.
- [x] **`TestRunDiscovery { email }` service-api request** - Invokes `rtsk::discovery::discover(email)` and returns the full `DiscoveredConfig` (options + diagnostics + oidc_endpoints) for harness assertions. `run_discovery_handle` in `crates/service/src/handlers/test_helpers.rs`; Lua dispatch in `crates/app/src/harness/mod.rs` maps `"TestRunDiscovery" | "test.run_discovery"`.
- [x] **`discovery-webfinger.lua`** - Happy-path script asserting the chained OIDC issuer resolves through WebFinger and the cascade returns the expected endpoints. `brokkr sync discovery-webfinger.lua` passes in ~3.4s.
- [x] **`discovery-oidc-bare-domain.lua`** - WebFinger absent (no fixture table), cascade falls back to the bare-domain `.well-known/openid-configuration` probe. Documented the fixture-level rule: bare-domain OIDC needs an absolute `issuer = "https://{domain}"` because that's the URL-space ratatoskr probes in (pre-rewrite), whereas the WebFinger-chained path operates in `${BASE}` space where path-relative issuers work.
- [x] **`discovery-autoconfig.lua`** - Mozilla autoconfig XML, `authentication="oauth2"` triggers `OAuth2Unsupported`, the cascade's post-merge upgrade against the OIDC discovery doc converts it to `OAuth2` with full endpoints. Required wiring `autoconfig.rs` through `rewrite_for_test_harness` (it was bypassing the shared helper).
- [x] **Negative-path coverage** - `discovery-negatives.lua` exercises malformed JRD (`raw_body`), non-HTTPS href in WebFinger response, and OIDC issuer self-claim mismatch from one fixture using multiple prefixes (cheaper than one spawn per case). Each path asserts `oidcEndpoints` stays absent. Oversized-body and redirect-chain-cap negatives remain unit-test only.
- [ ] **Saehrimnir follow-up: allow discovery-only fixtures.** The fixture loader currently requires at least one `[[account]]` even when only `[discovery]` tables are exercised. All four discovery fixtures carry a stub account to work around this; would be cleaner if discovery-only fixtures parsed without one.

### Environment-blocked (Windows)

The Linux equivalents already automate. The harness scripts are platform-agnostic; the gate is the test environment (cross-platform CI runner, dev box, or paid test service). If any of these become permanent automation, add Windows-capable Lua or libtest coverage and keep the Linux-only SIGTERM script separate.

- [ ] **M6.1 Windows parent-death (Job Object)** - Verify the Service exits when its parent is killed via the Windows Job Object machinery.
- [ ] **M6.2 Windows clean-shutdown handshake** - Verify SIGTERM-equivalent / `WM_CLOSE` triggers shutdown drain and the `clean_shutdown` sentinel.
- [ ] **M6.3 Windows stdio-corruption defense** - Verify `println!` from a handler doesn't corrupt JSON-RPC framing on Windows.

### M9 follow-ups (optional)

- [x] **Aux-quiesce the jmap/gmail steady-state scripts** - Resolved: both
  scripts carry the same quiesce block and post-quiesce baseline snapshot as
  the graph/imap twins, and both `plantasjen` baselines were re-recorded
  (`--as-baseline --bench 10`, provider_requests=2 both) and re-evaluated
  green against the new pins.

- [ ] **Per-host baselines for `jmap_steady_state_delta`** - The checked-in baseline map (`brokkr.toml`) is currently single-host (`plantasjen` only). Other contributors or CI hosts that should run the gate need to record their own baseline with `brokkr sync crates/app/tests/sync-harness/jmap-steady-state-delta.lua --gate jmap_steady_state_delta --as-baseline --bench 10` and append the printed line under `[ratatoskr.gate.jmap_steady_state_delta.baseline]`.
- [ ] **More checked-in gates** - Once a stable benchmark script matters to CI or release decisions, add a `[ratatoskr.gate.<name>]` block to `brokkr.toml` and record per-host baselines. Good candidates: JMAP scripted incremental, IMAP steady-state, Graph calendar remote-delta, CalDAV calendar remote-delta.
- [x] **`bifrost-consumer-hot-path`'s `meta.messages_per_second` rule has no
  throughput floor** - Resolved with the measured-absolute-floor option:
  benched 888.9 msg/s best-of-5 (the documented 228-331ms noise band maps to
  ~604-877), pinned `min = 300` with the rationale in `brokkr.toml`. An
  absolute floor was preferred over a `min_relative` (existence unverified
  upstream) because it survives a baseline repin - it cannot be blessed into
  a regression. Gate re-evaluated PASSED with the floor active, including a
  run at the slow end of the noise band (649 msg/s).

### Brokkr polish

- [ ] **`brokkr service --json`** - Machine-readable script discovery (the bare listing form of the collapsed `service` command; was `service-list --json` before brokkr `acd89f6`) for failure-triage tooling and editor integrations. Deferred (no current consumer).

### Capability backlog (land when a test needs it)

The original M1 foundation sketch named these as target surface; the M2-M8 cohort all landed without needing them. Each becomes work when a future test names coverage it unblocks.

- [ ] **Generic `harness.wait_for { predicate, child, backstop }`** - Lua-facing wait combinator that races arbitrary predicates against child-exit observation. Today's scripts use typed `ServiceClient` requests, event-stream receives, async request handles, and per-call timeouts.
- [ ] **`NotificationQueue` Lua userdata** - `queue:recv(timeout)` / `queue:drain_for(duration)` returning `Notification` userdata with `service_generation`, `method`, and a `serde_json::Value`-backed `params` view for filtering on payload details.
- [ ] **Sentinel-file watch** - `harness.wait_for_sentinel { path, backstop }` for data-dir-relative paths and `{ absolute, backstop }` for explicit absolute paths. No leading-slash auto-detection, no glob support.
- [ ] **Parent-death helper bindings** - `harness.spawn_parent_death_helper(service_binary, data_dir) -> { service_pid, helper_handle }`. The `parent_death_helper` binary already exists; the binding does not. Required for `linux_parent_sigkill_terminates_service_within_two_seconds`-style coverage.
- [ ] **Generic `harness.wait_exit(client, backstop) -> ExitStatus`** - With `code()`, `signal()`, `wall_time_ms()` accessors.
- [ ] **Resource-budget summary** - `harness.resource_summary(client) -> { rss_kb, io_bytes, ... }` reusing brokkr's existing sidecar profiler.
- [ ] **Parsed `frames.jsonl` payloads** - The frame writer currently records redacted raw frames + length + SHA-256 with `parsed: null`. Structural parsed redaction (per-`RequestParams` field allowlist) is future hardening before any credentialed script lands.

### Lua-helper cleanup

- [ ] **Hoist extract/search script helpers** - Don't add another extract/search script that copy-pastes backfill, attachment polling, search polling, or attachment lookup helpers. First hoist them into shared harness helpers or a supported Lua include path.

## Refactor Backlog

Flagged inline as `TODO(refactor)` with `#[allow(clippy::too_many_arguments)]` or `#[allow(clippy::type_complexity)]` so clippy stays clean. Nothing here is blocking - each is a localized API cleanup that would replace a long arg list or nested-Option tuple with a named struct.

**Replace long arg lists with a params struct:**
- [ ] `compose::new_reply` (8 args) - `crates/app/src/pop_out/compose/state.rs:253` -> `ReplyContext`
- [ ] `compose::build_recipient_row_inner` (8 args) - `crates/app/src/pop_out/compose/view.rs:388` -> recipient row params struct (autocomplete + selection state)
- [ ] `calendar_month::mini_month` (9 args) - `crates/app/src/ui/calendar_month.rs:348` -> navigation params struct
- [ ] `settings::row_widgets::slider_row` (9 args) - `crates/app/src/ui/settings/row_widgets.rs:528` -> `SliderRow` builder
- [ ] `undoable_text_input::handle_update` (8 args) - `crates/app/src/ui/undoable_text_input.rs:293` -> `UpdateCtx` struct

**Replace nested-Option tuples with named structs:**
- [ ] `merge_contact_pair_sync` builds a 6-tuple of `Option<String>` for the merge row - `crates/db/src/db/queries_extra/contacts/dedup.rs:65`. Local-only - immediately destructured into named locals; struct adds boilerplate without clarity gain. Skip unless we want zero `type_complexity` allows.
- [ ] compressed-body batches `(String, Option<Vec<u8>>, Option<Vec<u8>>)` (two call sites) - `crates/stores/src/body_store.rs:204, 360` -> `CompressedBody` struct. A local `type RawBodyRow` alias (line 301) handles one of the four similar shapes; the in-flight `Vec` shape and the row-read tuples remain. Skip unless we want zero `type_complexity` allows.

## Cross-Cutting Architecture Patterns

See `reference/architecture.md` § "Settled Patterns" for the living reference.
