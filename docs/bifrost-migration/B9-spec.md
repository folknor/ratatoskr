# B9 technical-implementation-spec: attachments plus cloud attachments

Rewires the two attachment seams onto bifrost. The INCOMING seam -
`attachment.fetch` and the background prefetch worker - stops calling the
per-provider `ProviderOps::fetch_attachment` and pulls bytes through the
already-resident bifrost `SyncEngine`'s blob openers (`open_blob` for the
HTTP providers, `open_raw_rfc822` + MIME re-parse for IMAP). The OUTGOING
seam - large-attachment cloud hosting (Google Drive, Microsoft OneDrive) -
retires ratatoskr's hand-rolled resumable-upload + share-link code
(`crates/gmail/src/gdrive.rs`, `crates/graph/src/onedrive.rs`) onto
bifrost's A6 `Account::host_attachment`, reached through a new
`SyncEngine::host_attachment` passthrough (B9-SQ, the one bifrost
side-quest this item needs).

**Feature-preserving mandate (governing principle, `docs/bifrost-migration.md`
§ 1).** B9 is a feature-preserving plumbing replacement. The rewire must
preserve each seam's CURRENT observable behavior exactly - neither
dropping a capability exercised today nor newly-wiring one that is not
wired today. This mandate is load-bearing for the pivotal survey
correction (§ 2.5): the OUTGOING cloud-upload subsystem is DEAD
SCAFFOLDING - `gdrive.rs`, `onedrive.rs`, the `cloud_attachments` upload
state machine, and the `CloudProvider` / `UploadStatus` types have NO
production caller anywhere in `service` / `app` / `core` (only the module
declaration `pub mod cloud_attachments`). B9 therefore does NOT wire a
new cloud-upload UX; it DELETES the hand-rolled duplicate of a bifrost
surface (§ 1 maximal-integration) and pins `SyncEngine::host_attachment`
as the canonical surface any future compose-side wiring consumes. The
INCOMING fetch path, by contrast, is live and user-facing; its wire ack
and byte pipeline are preserved byte-for-byte across the cut (§ 2.3).

This spec is written against `reference/technical-implementation-spec.md`
(the contract it must satisfy - READ IT) and conforms to its ten
clauses. It is one item of `docs/bifrost-migration.md` (the governing
plan and TODO source - READ § 3, § 4, § 5, § 7 B9 / A6, § 8, § 9, § 10,
§ 11), run through `reference/orchestrate.md`.

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick.
They are the ground this work is built on and judged against; naming
them is not enough.

- `reference/technical-implementation-spec.md` - the contract this spec
  is written against. The ten clauses below are its clauses.
- `reference/architecture.md` - ALWAYS required. Crate boundaries, the
  `core`/`app` firewall (the app depends on `rtsk` + `service-api` wire
  types only, never bifrost), the Service-as-stdio-child model, and the
  multi-store durability contract (main DB / body store / inline-image
  store / attachment file cache / search are separate durable stores)
  bind this structural change. B9 does NOT touch the `MailActionIntent`
  action pipeline (that was B4); it shares the Service boundary and the
  resident-engine handle B4a stood up.
- `docs/bifrost-migration.md` - the TODO source. § 1 (feature-preserving
  + maximal-integration: no hand-rolled duplicate of a bifrost surface
  survives), § 4 (shift 4, capability dispatch - `host_attachment` is
  capability-gated in bifrost, not per-provider-branched here), § 5
  (attachments is a "Rewired" seam), § 7 (B9 names B1 + A6 as
  prerequisites; the B3 / B3b / B4a done-notes for what is already
  resident), § 10 (behavioral gates are mandatory), § 11 (the frozen
  `../bifrost` commit discipline).
- `reference/glossary/harness.md` - the Service test harness,
  sync-harness scripts, `brokkr service-test` / `service-suite` /
  `sync-bench`, `saehrimnir` mock servers, and gate baselines. EVERY
  gate this spec pins (the `*-attachment-*` scripts) is defined there.
- `research/bifrost/reference/sync.md` - the `SyncEngine` read
  passthrough cluster this spec extends: `open_blob` / `open_blob_range`
  / `open_raw_rfc822` (added by the `dc670ef` side-quest, § "Read-only
  hydration passthroughs", `crates/sync/src/engine.rs:1466-1503`) plus
  `message_hydrate` / `thread_hydrate`. B9-SQ adds `host_attachment` to
  this cluster.
- `research/bifrost/reference/{google,graph,jmap,imap}.md` - the four
  per-provider blob surfaces (`blob.rs` / `open_blob` semantics, blob-id
  encoding, `open_raw_rfc822`) and, for Google/Graph, the A6 cloud
  hosting impls (`account/cloud.rs`).
- `research/bifrost/crates/types/src/blob.rs` + `.../cloud.rs` +
  `.../account.rs` (the `open_blob` / `open_raw_rfc822` / `host_attachment`
  trait methods, 219-237 / 419-423) + `.../capabilities.rs`
  (`host_attachment: bool`, 175; `blob_range: BlobRangeSupport`, 324) -
  the exact bifrost surface B9 consumes. Read the type definitions, not
  just the reference sheets.

The `../bifrost` dependency checkout is frozen at `cf024ab`
(`cf024ab...`, the B8-closing commit; the live `../bifrost` and
`./research/bifrost` both sit here) for the full duration of this item,
per `docs/bifrost-migration.md` § 11. **`../bifrost` (the build
dependency) and `./research/bifrost` (the reading reference this spec
cites by line number) are the same tree at the same commit `cf024ab`** -
every bifrost line-number citation below is pinned to that tree. Record
`cf024ab` in the ground survey of every sub-spec landing; do not let
`../bifrost` mutate underneath an in-flight step. (§ 11's freeze
narrative itemises side-quests through `a0a18c2` / B7a and does not yet
recount the B7b + B8 advances to `cf024ab`; that is the known § 11
reconciliation gap noted for B7a, not a discrepancy this spec
introduces. B9's B9-SQ, § 3, advances the freeze from `cf024ab` and is
recorded there at land.)

## 1. The goal (clause 7: the target as concrete artifacts)

Today ratatoskr owns BOTH attachment seams in hand-rolled per-provider
code:

- INCOMING (live). `attachment.fetch`
  (`crates/service/src/handlers/attachment.rs:202`) and the prefetch
  worker (`crates/service/src/prefetch.rs:1144` / `:1184`) build a
  `Box<dyn ProviderOps>` via `create_provider` and call
  `provider.fetch_attachment(ctx, message_id, provider_attachment_id)
  -> FetchedAttachment { bytes, size }`
  (`crates/common/src/ops.rs:128`), except IMAP which calls
  `imap::client::fetch_attachment_on_selected`. The bytes then flow
  through the UNCHANGED cache pipeline: optional squeeze compression ->
  `PackStore::put` -> `BlobHash` -> `update_attachment_cache_fields` ->
  `materialize_blob` -> wire ack `{ content_hash, size_bytes,
  relative_path }`.
- OUTGOING (dead scaffolding, § 2.5). `crates/gmail/src/gdrive.rs` and
  `crates/graph/src/onedrive.rs` implement Google Drive / OneDrive
  resumable upload + share-link creation, driven by the
  `cloud_attachments` upload state machine
  (`crates/db/src/db/queries_extra/cloud_attachments.rs`,
  `crates/core/src/cloud_attachments.rs` `UploadStatus` / `CloudProvider`).
  NO production path in `service` / `app` / `core` calls any of it.

B9 rewires both onto bifrost (`docs/bifrost-migration.md` § 7 B9, § 5
"Rewired"). After B9:

- INCOMING bytes come from the resident bifrost `SyncEngine`:
  - HTTP providers (Gmail, Graph, JMAP): reconstruct a
    `bifrost_types::BlobHandle` from the attachment row's persisted
    verbatim blob id (§ 2.4) and call
    `engine.open_blob(account, handle) -> AccountStream<SyncEvent<Bytes>>`
    (`crates/sync/src/engine.rs:1482`), draining the stream to the full
    payload.
  - IMAP: the byte source is the assembled message -
    `engine.open_raw_rfc822(account, message ObjectId)` (`engine.rs:1470`),
    MIME-re-parsed to extract the part by its stored `part_id`, mirroring
    the B3 consumer's IMAP hydrate re-parse (`bifrost/consumer/hydrate.rs`).
    NOTE (R1/R2 correction): this is NOT because bifrost IMAP lacks a
    per-part blob fetch - bifrost IMAP DOES implement `open_blob`, and it
    is a real single-part `BODY.PEEK[section]` fetch
    (`research/bifrost/crates/imap/src/account/blob.rs:15`,
    `mod.rs:381`, `reference/imap.md:275`). The reason IMAP routes through
    `open_raw_rfc822` is that ratatoskr's OWN B3 consumer stores only the
    re-parsed `part_id` for IMAP attachments (`hydrate.rs:684-699`), never
    a bifrost `BlobHandle.id` (folder/uidvalidity/uid/section), so there is
    no handle to hand `open_blob`. This is a ratatoskr-persistence
    constraint, not a bifrost capability gap, and it carries a
    single-fetch tradeoff (§ 2.3, § 4.1.2) - it is not "strictly better."
    An alternative that avoids the tradeoff (persist the bifrost IMAP
    `BlobHandle.id` alongside `part_id` and route IMAP to `open_blob` too)
    is recorded in § 9 finding A/B.
  - The bytes then flow through the SAME squeeze / `PackStore` / cache /
    materialize / wire-ack pipeline, UNCHANGED. Only the byte SOURCE moves
    from `ProviderOps::fetch_attachment` to the engine blob opener.
- OUTGOING hosting is bifrost's A6 surface: `Account::host_attachment(bytes,
  CloudUploadMeta) -> HostedAttachment { share_url, provider_file_id }`
  (`types/src/account.rs:419`, capability-gated
  `pim_methods.host_attachment`, `true` only on Google -> Drive and Graph
  -> OneDrive), reached through the new `SyncEngine::host_attachment`
  passthrough (B9-SQ). The hand-rolled `gdrive.rs` / `onedrive.rs` /
  upload-state-machine duplicate is DELETED (§ 1 maximal-integration).
- The account attach requirement: the engine blob/host passthroughs
  resolve through `live_account` and return `AccountNotAttached` when the
  account is not resident-attached. B9 reaches the resident slot exactly
  as B4a's `ResidentActionAccount` does (`bifrost/resident.rs`),
  attaching on demand if the account is idle (§ 4.1.2).

The target seam, pinned to concrete types:

```
attachment.fetch / prefetch (service, unchanged wire ack)
  -> AttachmentByteSource (NEW, crates/service/src/bifrost/attachment.rs)
       per account provider-kind:
         HTTP (gmail/graph/jmap):
           BlobHandle { id: BlobId(row.blob_id), size, content_type,
                        digest: None, capabilities: <open_blob defaults> }
           -> engine.open_blob(account, handle) -> drain SyncEvent<Bytes>
           -> Vec<u8>
           (stale-handle fallback for JMAP: re-hydrate via
            engine.message_hydrate to mint a fresh handle, § 2.4)
         IMAP:
           engine.open_raw_rfc822(account, ObjectId) -> drain -> MIME
           re-parse -> extract part by part_id -> Vec<u8>
  -> maybe_compress -> PackStore::put -> update_attachment_cache_fields
  -> materialize_blob -> AttachmentFetchAck (UNCHANGED)

compose/send cloud hosting (future caller; surface pinned here)
  -> engine.host_attachment(account, bytes, CloudUploadMeta) (B9-SQ)
       -> live_account.host_attachment -> HostedAttachment
```

`FetchedAttachment` (`common/src/types.rs:72`) - the `{ bytes, size }`
DTO the deleted `fetch_attachment` returned - is no longer the seam; the
byte source returns `Vec<u8>` directly and the handler computes size from
it, exactly as today's post-fetch code does. The hand-rolled cloud types
(`CloudProvider`, `UploadStatus`, `GDriveUploadSession`, the OneDrive
session types) are deleted; `bifrost_types::{CloudUploadMeta,
HostedAttachment, ShareScope}` replace them.

## 2. Survey of the ground (clause 8)

### 2.1 What earlier items already laid (additive, reused)

- **The resident engine + read passthroughs (B3b, B4a).**
  `crates/service/src/bifrost/resident.rs` holds each account `attach`ed
  across kicks (`ResidentEngine` / `ResidentSlot`). B4a added
  `ResidentActionAccount`, the per-account handle the action pipeline
  drives mutations through, resolved from the resident slot. B9's byte
  source reaches the engine the same way. `BifrostSyncEngine::engine()`
  (`bifrost/engine.rs:31`) returns `Arc<SyncEngine>`, which exposes
  `open_blob` / `open_blob_range` / `open_raw_rfc822`
  (`crates/sync/src/engine.rs:1466-1503`, the `dc670ef` passthrough
  cluster) and `message_hydrate` / `thread_hydrate`. The engine is built
  at boot (`boot.rs:1449`) and installed on the `SyncRuntime`.
- **The persisted attachment rows (B3 consumer).**
  `bifrost/consumer/hydrate.rs` writes `AttachmentInsertRow`
  (`db/queries_extra/message_persistence.rs:43`) for every synced
  attachment: `id` (`"{message_id}_{blob.id.0}"`), `remote_attachment_id`,
  `content_id`, `mime_type`, `size`, `is_inline`, and (for inline images)
  `content_hash`. This is the row `attachment.fetch` reads via
  `find_attachment_cache_info` to decide cache-hit vs. cache-miss. B9 adds
  one column to it (§ 2.4).
- **B1 error mapping.** `bifrost/error_map.rs`
  (`account_error_to_operation_result` / `_to_action_error`) maps a failed
  `open_blob` / `host_attachment` `AccountError` down to the wire
  `ServiceError` / `OperationResult`, reused verbatim.
- **A6 in bifrost (frozen `cf024ab`).** `Account::host_attachment`
  (`types/src/account.rs:419`) + `CloudUploadMeta` / `HostedAttachment` /
  `ShareScope` (`types/src/cloud.rs`) + the `host_attachment` capability
  flag (`types/src/capabilities.rs:175`) + the Google
  (`google/src/account/cloud.rs`) and Graph
  (`graph/src/account/cloud.rs`) resumable-upload + share-link impls. This
  is exactly the OUTGOING surface `gdrive.rs` / `onedrive.rs` duplicate.

### 2.2 What B9 rips out

- **INCOMING routing (rewired, not the trait yet).** The
  `create_provider(...).fetch_attachment(...)` calls at
  `handlers/attachment.rs:192-205` and `prefetch.rs:1128-1155`, and the
  `imap::client::fetch_attachment_on_selected` call at `prefetch.rs:1184`.
  B9 removes these fetch CALLERS. The `ProviderOps::fetch_attachment` trait
  method (`common/src/ops.rs:128`), its four provider impls
  (`gmail/graph/jmap/imap/src/ops.rs`), `FetchedAttachment`, and
  `create_provider` itself (which still backs other reads) RETIRE at B15,
  not here - consistent with the B3a-cut-imap disposition of the surviving
  action-ops factory. B9 must leave `create_provider` compiling (it has
  other callers) while removing the attachment ones.
- **OUTGOING hand-rolled duplicate (deleted, § 1 maximal-integration).**
  `crates/gmail/src/gdrive.rs` (Drive resumable upload + sharing) and
  `crates/graph/src/onedrive.rs` (OneDrive resumable upload + sharing) -
  the exact duplicate of A6 `host_attachment`. The upload half of
  `crates/core/src/cloud_attachments.rs` (`UploadStatus`, the upload-state
  parts of `CloudProvider`) and the `cloud_attachments` upload state
  machine (`db/queries_extra/cloud_attachments.rs`, the outgoing-direction
  rows and their status/session/bytes-uploaded columns). All of it is
  dead (§ 2.5), so deletion strands no live behavior.

### 2.3 The INCOMING fetch path, preserved exactly (clause 8: the load-bearing work the rip must not change)

`attachment.fetch` (`handlers/attachment.rs`) is a live, user-facing
critical path. Its shape MUST survive the cut unchanged except for the
byte source:

- Cache-hit branch (`:129-177`): content_hash present + `attachment_blobs`
  row live + not tombstoned -> `materialize_blob` (or the inline-image
  store for `is_inline`) -> ack. NO provider/engine call. UNCHANGED.
- Cache-miss branch (`:179-268`): fetch bytes -> `maybe_compress`
  (settings-gated squeeze, non-fatal) -> `PackStore::put` -> `content_hash`
  -> `update_attachment_cache_fields` -> `materialize_blob` -> enqueue text
  extraction -> ack `{ content_hash, size_bytes, relative_path }`. B9
  replaces ONLY the "fetch bytes" step (`:202-205`).
- The account-is-deleting short-circuit (`:83-101`), the
  inline-image-store fallback, the extraction-enqueue gate, and the
  tmp-cleanup / cache-size / clear-cache handlers are all UNCHANGED.
- Prefetch (`prefetch.rs`): the background worker's HTTP-provider fetch
  (`:1144`) and IMAP `fetch_attachment_on_selected` (`:1184`, which reuses
  a selected IMAP session for batched part fetches) both rewire onto the
  engine byte source. CAUTION (R2 finding B): the "whole message once, N
  attachments hydrate once not N times" claim is NOT delivered by a naive
  per-item rewire. Today's prefetch loop invokes the item pipeline once per
  attachment (`prefetch.rs:1011`) over one reused IMAP session (one SELECT,
  N per-part `BODY[part]` fetches - the `imap-folder-batch-session-reuse.lua`
  gate asserts exactly 3 body fetches on 1 connection, `:200-217`). If each
  item's `AttachmentByteSource::fetch` calls `open_raw_rfc822`
  independently, an N-attachment message performs N FULL RFC822 downloads +
  re-parses - a REGRESSION, not an improvement. "Once per message" requires
  a message-level batching artifact (hydrate the RFC822 once and satisfy
  all of that message's queued parts from the single parse) that this spec
  must specify, OR the IMAP `open_blob`-with-persisted-handle alternative
  (§ 9 finding A/B). The single-fetch (live `attachment.fetch` cache-miss
  of one part of a large multi-attachment message) is a strict regression
  vs today's single-part fetch either way; name it as a tradeoff.

Wire types (`service-api`) do NOT change: `AttachmentFetchParams` in,
`AttachmentFetchAck` out. The `core`/`app` firewall holds - no bifrost
type reaches `service-api` / `core` / `app` (`AttachmentByteSource` and
every bifrost type it touches live inside `service`).

### 2.4 The BlobHandle reconstruction obstacle (clause 2: resolved inline)

`Account::open_blob` takes a `BlobHandle { id: BlobId, size, content_type,
digest, capabilities }` (`types/src/blob.rs:71`), and each provider's
`open_blob` DECODES `BlobHandle.id` (an opaque, provider-encoded string -
Gmail/Graph encode a JSON `{message_id, attachment_id}`; JMAP a server
blob id) to issue the download. So the fetch path needs the VERBATIM
bifrost blob id at fetch time.

**The obstacle:** the B3 consumer does NOT persist the verbatim blob id in
a clean, dedicated field. `hydrate.rs::remote_attachment_id(provider,
blob_id)` (`:800`) UNWRAPS the Graph/Gmail JSON to just the inner
`attachment_id` before storing it in `attachments.remote_attachment_id`.
Reconstructing the exact `BlobHandle.id` from that lossy value is not
possible for the HTTP providers. (R1 SMELL, finding I: the verbatim
`blob.id.0` IS in fact already embedded in `attachments.id`, which the
consumer builds as `format!("{}_{}", message_id, blob.id.0)` at
`hydrate.rs:594`. A dedicated column is still chosen over splitting `id` on
`_`, because `message_id` itself can contain `_`, making the split
ambiguous - but the spec must not claim the id is unrecoverable; it is
recoverable-but-ambiguous, and the new column is the clean fix.)

**Resolution (pinned, ratatoskr-side, no bifrost dependency):** add a
`blob_id TEXT` column to the `attachments` table
(`db/src/db/schema/02_mail.sql:276`), a v100 schema EDIT (pre-release,
no-v101 policy - `crates/db/src/db/migrations.rs`), written by
`hydrate.rs::build_consumer_row` with the VERBATIM `blob.id.0` (the string
before `remote_attachment_id`'s unwrap). At fetch, `AttachmentByteSource`
reads `blob_id` and builds `BlobHandle { id: BlobId(blob_id), size:
row.size, content_type: row.mime_type, digest: None, capabilities:
BlobCapabilities { supports_range: false, supports_parallel: false,
digest_available_pre_download: false, encoding: <provider default> } }`.
Only `id` is load-bearing for `open_blob` (the capability flags gate
`open_blob_range`, which the whole-blob fetch does not use), so defaulting
them is sound. `remote_attachment_id` stays (it feeds
`find_attachment_cache_info`'s lookup and the IMAP `part_id`); `blob_id`
is additive.

**JMAP stale-handle fallback (REWORK REQUIRED - R2 finding E).** The
originally-sketched fallback does not survive review and must be reworked
or dropped before implementation. Recorded here so the hole is not carried
forward as if resolved:

- API signature was wrong. `message_hydrate` takes ONE `ObjectId` plus a
  `HydrationProjection`, returning a `Message` (`engine.rs:1436`) - NOT
  `[ObjectId]` and NOT a `Projection::Metadata-with-blobs` (which does not
  exist).
- No stable discriminator to match the replacement. A re-hydrated `Message`
  exposes attachments only as `Vec<BlobHandle>`; the persisted row carries
  old blob id, MIME, size, filename, but no stable part id or ordinal, and
  two attachments can share MIME + size, so "match the one whose part
  corresponds to this attachment" is ambiguous. Fixing this needs a persisted
  stable attachment discriminator (e.g. the attachment ordinal / part index
  captured at hydrate time) or a proven ordinal-matching contract.
- Feature-parity rationale was false. Legacy JMAP `fetch_attachment` does a
  DIRECT `client.download(attachment_id)` (`jmap/src/ops.rs:535`) - it does
  NOT re-derive the blob from a fresh `Email/get`. So this fallback ADDS
  behavior; it does not preserve it, and § 1's feature-preserving mandate
  cuts against adding it.
- Its proposed harness gate needs a `saehrimnir` blob-rotation affordance
  that does not exist today.

Resolution options (pin one at land, do not leave open): (a) persist a
stable attachment discriminator at hydrate time and specify exact ordinal
matching + build the `saehrimnir` rotation affordance; or (b) DROP the
newly-added JMAP re-mint fallback entirely (feature-preserving default),
surfacing an expired-blob `open_blob` failure as the same error the legacy
direct download would surface. Option (b) is the § 1-aligned default unless
a concrete expiry regression is demonstrated.

**Reconciliation with the NULL-`blob_id` trigger (R1 finding, § 4.1.1).**
§ 4.1.1 makes a NULL `blob_id` on ANY HTTP-provider row a re-hydrate
trigger (for rows synced before the column existed). That is effectively an
all-provider re-hydrate path, contradicting a "JMAP-only" framing:
Gmail/Graph pre-column rows must ALSO re-hydrate. Since dev-seed wipes and
re-seeds every launch and this is pre-release, the simpler disposition is to
rely on a full re-sync repopulating `blob_id` for every provider and NOT
build a per-fetch re-hydrate path at all (pairs with option (b) above). If
a re-hydrate path is kept, it must be specified as all-provider, not
JMAP-only.

### 2.5 The OUTGOING cloud subsystem is DEAD SCAFFOLDING (the pivotal survey finding)

A direct survey of every `service` / `app` / `core` caller establishes
that the entire outgoing cloud-attachment subsystem has NO production
consumer:

- `crates/gmail/src/gdrive.rs` / `crates/graph/src/onedrive.rs`: grep for
  their public fns (`create_upload_session`, `upload_file_chunked`,
  `create_sharing_permission`, ...) finds ZERO callers outside their own
  test modules.
- `db/queries_extra/cloud_attachments.rs` (`insert_cloud_attachment`,
  `update_cloud_attachment_status`, ...): the ONLY references are the
  `queries_extra.rs` re-export and the file itself. No `service` / `app`
  handler drives the upload state machine.
- `core/src/cloud_attachments.rs`: the only reference to the module from
  outside is `pub mod cloud_attachments` in `core/src/lib.rs:8`.
  `supports_cloud_upload` / `UploadStatus` / the `CloudProvider` upload
  variants have no live caller.
- `graph/src/ops/send.rs` handles only inline (base64) attachments; it
  does NOT invoke OneDrive hosting.

**Consequence for B9 (feature-preserving mandate, § 1).** B9 does NOT
build a cloud-upload UX - that would newly-wire a feature absent today,
violating § 1. B9's outgoing work is (a) DELETE the hand-rolled duplicate
of the bifrost A6 surface (§ 1 maximal-integration: no parallel
hand-rolled equivalent survives), and (b) PIN `SyncEngine::host_attachment`
(B9-SQ) as the single, capability-gated surface a future compose-side
item wires, with the concrete call artifact specified here (§ 4.2) so
that item inherits a built road, not a design gap. The `cloud_attachments`
table's OUTGOING columns retire; if the compose UI later needs an upload
journal, it is minted fresh against the bifrost surface, not revived from
the dead machine.

### 2.6 Incoming cloud-LINK detection / enrichment (carve-out, no bifrost equivalent)

`core/src/cloud_attachments.rs` also holds INCOMING link handling -
`detect_cloud_links` (scans an HTML body for Drive/OneDrive/Dropbox/Box
share URLs) and `enrich_onedrive_link` / `enrich_gdrive_link` (resolve a
received share URL's file metadata via `GET /shares/{}/driveItem` /
`GET /drive/v3/files/{id}`). This is BODY scanning + public-share metadata
resolution, NOT an `Account` mail operation, and bifrost has NO equivalent
surface (A6 covers outbound hosting only). These are ALSO currently
unwired (§ 2.5). Disposition: they are a SEPARATE concern, carved OUT of
B9 - B9 neither rewires nor deletes them beyond removing the shared
upload-only types they no longer compile against. The enrichment fns'
direct-provider-API calls (`GraphClient` / `reqwest` + token) are a § 1
wart with no bifrost home; that carve-out is named here (not deferred as a
hole) and left for a future item should the incoming-link feature ever be
wired. (`detect_cloud_links` is pure body-regex and needs no provider at
all.)

### 2.7 The cursor / table disposition

B9 touches no sync cursor. The `attachments` table gains one additive
`blob_id` column (§ 2.4). The `cloud_attachments` table is KEPT (decided
now, not at land - R2 finding H): its incoming writer
`insert_incoming_cloud_links_sync` is part of the § 2.6 carve-out, so the
table cannot be dropped. Only the OUTGOING upload writers and their
status/session/bytes-uploaded columns retire with the dead machine
(§ 2.5); the outgoing columns may be dropped in the v100 edit or left inert
(no writer) if a column-drop is not worth the churn - either way no orphaned
writer remains. This supersedes the earlier "pinned per sub-spec at land"
table deferral.

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

Three ordered landings. Each is one coherent, fully intrusive change,
kept or reverted on its gates. `brokkr check` is green at every boundary.

### B9-SQ - bifrost `SyncEngine::host_attachment` passthrough (side-quest, lands first)

`Account::host_attachment` exists at `cf024ab` but `SyncEngine` exposes NO
`host_attachment` passthrough (only the `dc670ef` read cluster and the
`75cf810` / `8ea29b6` mutation/compose clusters). B9's outgoing rewire
cannot reach the engine-private `Arc<dyn Account>` without it. Per the § 2
side-quest protocol, ONE Opus agent adds
`SyncEngine::host_attachment(account_id, bytes, meta) -> Result<..,
Error>` in `crates/sync/src/engine.rs`, resolving through `live_account`
(bailing `AccountNotAttached` up front) and forwarding to
`Account::host_attachment`, mirroring the existing passthrough clusters
verbatim in shape. It is a pure additive forwarder - no new protocol
work, since Google/Graph `account/cloud.rs` already implement the trait
method. The orchestrator commits it in `./research/bifrost`, runs the
bifrost side-gate (`brokkr check` there), and promotes via
`bash scripts/bifrost.sh`, advancing the freeze from `cf024ab` (recorded
in § 11 at land). B9a and B9b then pin the advanced commit.

### B9a - INCOMING fetch rewire (open_blob / open_raw_rfc822)

The user-facing landing, independently landable and shippable, needing NO
bifrost change (the `open_blob` / `open_raw_rfc822` passthroughs exist at
`cf024ab`). Adds the `blob_id` column + consumer persist (§ 2.4), builds
`AttachmentByteSource` (§ 4.1), rewires `attachment.fetch` + prefetch onto
it, and removes the `fetch_attachment` / `fetch_attachment_on_selected`
callers in the SAME landing. Gated GREEN by the full `*-attachment-*`
sync-harness suite (§ 6.1). B9a does NOT depend on B9-SQ or B9b.

### B9b - OUTGOING host surface + dead-scaffolding deletion

On top of a green B9-SQ. Pins the `engine.host_attachment` call artifact
(§ 4.2), deletes `gdrive.rs` / `onedrive.rs` / the upload state machine /
the dead `cloud_attachments` outgoing surface (§ 2.5), and retires the
outgoing table columns. No user-visible behavior changes (the deleted code
had no caller); the gate is `brokkr check` green + the
compile-and-capability unit test for the pinned surface (§ 6.2).

**Pinned landing order (R1/R2 nit - was contradictory).** The prior text
said both "B9-SQ lands first" (subsection heading) and "a sound landing
order is B9a first." Resolved to ONE unambiguous sequence: **B9a, then
B9-SQ, then B9b.** B9a is the live user-facing feature and needs no bifrost
change, so it lands first; B9-SQ (the bifrost passthrough) lands second;
B9b (outgoing surface + deletion) lands last on top of B9-SQ. The "lands
first" phrase in the B9-SQ subsection heading refers to B9-SQ preceding B9b
(its only dependant), not preceding B9a. Each landing is independently
green; none regresses another.

## 4. The bricks

### 4.1 B9a - the incoming byte source

#### 4.1.1 The `blob_id` column + consumer persist

- Schema: add `blob_id TEXT` to `attachments`
  (`db/src/db/schema/02_mail.sql`, the v100 migration in
  `db/src/db/migrations.rs`). Additive; no index needed (looked up by the
  existing `id` / `remote_attachment_id` path, then `blob_id` read off the
  resolved row).
- `AttachmentInsertRow` (`db/queries_extra/message_persistence.rs:43`)
  gains `pub blob_id: Option<String>`; `insert_attachments` (`:112`) writes
  it. This requires updating THREE places, not one (R2 finding G, R1 nit):
  (1) the `INSERT ... VALUES` + `ON CONFLICT DO UPDATE` statement at
  `message_persistence.rs:115-135`; (2) the in-test `CREATE TABLE
  attachments` at `message_persistence.rs:153-164`, or the
  `attachment_upsert_does_not_clear_existing_content_hash_with_null` unit
  test breaks; and (3) the SECOND `AttachmentInsertRow { .. }` construction
  site the § 2 survey missed - the legacy Graph provider-sync writer at
  `crates/provider-sync/src/graph/sync/persistence.rs:349`. A struct-literal
  field is mandatory at every site even when `Option`, so this writer will
  fail to compile until updated. Disposition: it writes legacy Graph rows
  with `blob_id: None` (no bifrost handle available on that path); those
  rows then depend on whatever fallback § 2.4 pins for a NULL `blob_id`
  (re-sync repopulation under the bifrost consumer). Whether the legacy
  provider-sync Graph path is still live under B9 must be stated at land.
- `hydrate.rs::build_consumer_row` sets `blob_id: Some(blob.id.0.clone())`
  (the VERBATIM bifrost blob id, before the `remote_attachment_id` unwrap).
  For the IMAP re-parse branch (`:684`), `blob_id` is `None` (IMAP has no
  server blob; the byte source routes IMAP to `open_raw_rfc822`).
- Because dev-seed wipes + re-seeds every launch and this is pre-release
  (no migration of existing user rows), a full re-sync repopulates
  `blob_id`; the byte source treats a NULL `blob_id` on an HTTP-provider
  row as a re-hydrate trigger (§ 2.4 fallback), so a row synced before the
  column existed still fetches.

#### 4.1.2 `AttachmentByteSource` (concrete artifact)

New module `crates/service/src/bifrost/attachment.rs`:

```rust
pub struct AttachmentByteSource {
    engine: Arc<bifrost_sync::SyncEngine>,  // via BifrostSyncEngine::engine()
    resident: Arc<ResidentEngine>,          // for attach-on-demand
}

impl AttachmentByteSource {
    /// Pull the full decoded bytes for one attachment. Preserves the
    /// legacy fetch_attachment contract: returns Vec<u8>, the caller
    /// computes size + hashes as today.
    pub async fn fetch(
        &self,
        account_id: &str,
        provider: BifrostProviderKind,
        message_id: &str,
        row: &AttachmentFetchRow,  // blob_id, remote_attachment_id (part_id), imap ids
    ) -> Result<Vec<u8>, AttachmentByteError>;   // NOT ServiceError - see below
}
```

**Error type (R2 finding C - CORRECTION).** The return type is NOT
`Result<Vec<u8>, ServiceError>`. Two problems with `ServiceError`: (1) the
prefetch worker classifies failures into `SkipReason::{ProviderTimeout,
ProviderTransient, ProviderPermanent, InternalError}` and drives a
timeout-fed circuit breaker off `ProviderTimeout`
(`prefetch.rs:190-214`, `is_timeout`); `ServiceError` cannot preserve those
classes. (2) The cited `bifrost/error_map.rs` mapper returns
`OperationResult` / `ActionError`, NOT `ServiceError` (`error_map.rs:5-25`).
So `AttachmentByteSource::fetch` must return a typed byte-source error
(`AttachmentByteError`) carrying at least: engine `Error` /
`AccountNotAttached`, `AccountError` (with its `RecoveryClass`), timeout,
missing-part (IMAP re-parse found no matching `part_id`), and
non-byte-stream (Graph reference attachment, below). The handler maps it to
its `ServiceError::Internal` wire shape; the prefetch worker maps it to the
correct `SkipReason` (timeout -> `ProviderTimeout`, `AccountError`
recovery-class -> `Transient`/`Permanent`, internal -> `InternalError`).
Mapping happens at each caller, not inside the byte source.

- Attach-on-demand: `open_blob` / `open_raw_rfc822` resolve through
  `live_account` and error `AccountNotAttached` off the resident slot. If
  the account is not attached (idle, no resident kick in flight), `fetch`
  calls `resident.attach_account(account_id)` first, mirroring
  `ResidentActionAccount`'s attach path (B4a). Under the B3b keep-attached
  lifecycle a synced account is normally already resident, so the on-demand
  attach is the cold-idle edge only.
- HTTP dispatch: build the `BlobHandle` from `row.blob_id` (§ 2.4), call
  `engine.open_blob(account, handle)`, and drain the
  `AccountStream<SyncEvent<Bytes>>` - accumulating `SyncEvent::Batch`
  bytes, honoring `SyncEvent::Terminated(err)` as the failure (mapped to
  `AttachmentByteError`), completing on `SyncEvent::Done`. (Note: `account`
  is a `&AccountId`; the § 4.2.1 sketch's `account_id.into()` is loose -
  the frozen `AccountId` does not take an `&str` `.into()` at these call
  sites, R2 finding F.)
- Graph reference-attachment guard (R2 finding D - CORRECTION). Draining
  cannot treat "stream ended without `Terminated`" as success-with-bytes.
  Graph's `open_blob` emits `SyncEvent::Warning(BlobNotByteStream)` then
  `SyncEvent::Done` with NO byte batch for reference attachments and for a
  `405 Method Not Allowed` fetch
  (`research/bifrost/crates/graph/src/account/blob.rs:121-125, 151-155`).
  The naive drainer would return an EMPTY `Vec`, `PackStore::put` it, and
  ack success - caching an empty file. The drainer MUST detect
  `SyncEvent::Warning(BlobNotByteStream)` and surface it as a distinct
  `AttachmentByteError` variant (non-byte-stream), which the handler treats
  as "no downloadable bytes" rather than a zero-byte success. A legitimate
  zero-byte attachment (a real `Batch` of zero-length bytes ending in
  `Done`) stays distinct from a warning-terminated non-byte-stream. Needs a
  Graph reference-attachment harness gate (§ 6.1).
- JMAP stale-handle: see § 2.4 - the re-mint fallback is REWORK-REQUIRED
  (bad API, no discriminator, false parity rationale) and defaults to being
  dropped; do not implement the sketched `message_hydrate` re-mint until
  § 2.4's option (a) or (b) is pinned.
- IMAP dispatch: reconstruct the message `ObjectId`
  (`imap1:<len>:<folder>:<uidvalidity>:<uid>` - the same reconstruction
  B4a's `resolve_thread_messages` does from the stored ids), call
  `engine.open_raw_rfc822(account, ObjectId)`, drain to the full RFC822
  bytes, MIME-re-parse (the same parser the consumer's IMAP hydrate uses),
  and extract the part whose `part_id` matches `row.remote_attachment_id`.
  A `part_id` miss is an `AttachmentByteError` (missing-part), never a
  silent empty return. PREFETCH BATCHING (R2 finding B): a per-item
  `open_raw_rfc822` call means an N-attachment message downloads + re-parses
  its full RFC822 N times - worse than today's session-reuse. To honor
  "once per message" the prefetch path needs a message-level cache/dedup
  (hydrate one message's RFC822 once, satisfy all its queued parts from the
  single parse), specified as a concrete artifact here, or the
  persist-IMAP-`BlobHandle` alternative (§ 9 A/B). This is NOT optional
  polish - the existing `imap-folder-batch-session-reuse.lua` gate
  (3 body fetches / 1 connection) fails otherwise (§ 6.1).
- Errors map through `bifrost/error_map.rs` to `ServiceError::Internal`
  (the shape the handler already returns for a failed provider fetch), so
  the wire-error surface is unchanged.

#### 4.1.3 Rewire the callers

- `handlers/attachment.rs:192-205`: replace the `create_provider` +
  `provider.fetch_attachment` block with an `AttachmentByteSource::fetch`
  call; the returned `Vec<u8>` feeds `maybe_compress` exactly as
  `attachment.bytes` does today. Everything else in the handler is
  untouched.
- `prefetch.rs:1128-1155` and `:1184`: same replacement; the HTTP and
  IMAP prefetch branches collapse to one `AttachmentByteSource::fetch`
  call. The `AttachmentByteSource` is constructed once and shared (it
  holds only `Arc`s).
- The handler/prefetch reach `AttachmentByteSource` through the shared
  service state that already holds the `BifrostSyncEngine` /
  `ResidentEngine` (the same handle the runner and B4a's action dispatch
  use); expose it on `BootSharedState` if not already reachable.

### 4.2 B9b - the outgoing host surface + deletion

#### 4.2.1 The pinned `host_attachment` call artifact

The one concrete artifact B9b leaves for the future compose caller (built,
not merely directional):

```rust
// crates/service/src/bifrost/attachment.rs (same module)
pub async fn host_large_attachment(
    engine: &SyncEngine,
    account_id: &str,
    bytes: bytes::Bytes,
    file_name: &str,
    mime: &str,
    scope: ShareScope,          // Anyone | Organization
) -> Result<HostedAttachment, ServiceError> {
    let meta = CloudUploadMeta::new(file_name, mime, bytes.len() as u64, scope);
    engine.host_attachment(account_id.into(), bytes, meta)   // B9-SQ passthrough
        .await
        .map_err(/* error_map -> ServiceError */)
}
```

Capability dispatch is bifrost's (§ 4 shift 4): a JMAP/IMAP account
returns `Unsupported(HostAttachment)`, mapped to a clean capability-absent
error the caller reads declaratively - never a per-provider `match` in
ratatoskr. The 25 MB threshold, warn-vs-host UX, and body link-insertion
(the parts § 1 says the consumer owns) are NOT built here (no caller
today); the surface is what B9b pins.

**Sketch corrections (R2 finding F, R1 nit).** The sketch above is
directional, not literal, and has four defects the built artifact must fix:

1. Attach-on-demand is impossible with `&SyncEngine` alone. § 1 promises
   cold accounts attach on demand for hosting, but `engine.host_attachment`
   resolves through `live_account` and returns `AccountNotAttached` for an
   idle account. Like `AttachmentByteSource` (§ 4.1.2), this artifact needs
   the `ResidentEngine` handle to attach first. Signature must carry it
   (mirror `AttachmentByteSource`'s `engine` + `resident`), not a bare
   `&SyncEngine`.
2. `account_id.into()` is not supported by the frozen `AccountId` type at
   this call site; pass a real `&AccountId`.
3. The error is NOT `ServiceError`. `ServiceError` has no capability /
   unsupported variant, so "capability-absent `ServiceError` the UI reads
   declaratively" is impossible. Return a typed ratatoskr error (same
   family as `AttachmentByteError`, § 4.1.2) that carries the
   `Unsupported(HostAttachment)` case distinctly.
4. Two-layer error shape. The B9-SQ passthrough mirrors the other engine
   passthroughs' shape: `Account::host_attachment` returns
   `AccountFuture<Result<HostedAttachment, AccountError>>`, so the engine
   method is `pub fn host_attachment(..) -> Result<AccountFuture<Result<
   HostedAttachment, AccountError>>, Error>` - `AccountNotAttached` surfaces
   synchronously via the outer `?`, `AccountError` via the awaited inner
   `Result`. The one-`.map_err` collapse in the sketch drops the outer `?`.

Given (1)-(3), if no compose caller exists to define the wire contract, the
alternative disposition is to NOT ship a standalone `host_large_attachment`
helper in B9b and instead pin ONLY the B9-SQ engine passthrough (§ 3) plus
the capability unit test (§ 6.2), leaving the resident-handle-bearing
service wrapper to the future compose item that defines the error surface.
Pin which of the two at land.

#### 4.2.2 Deletion

- Delete `crates/gmail/src/gdrive.rs` and `crates/graph/src/onedrive.rs`
  and their `mod` declarations.
- Delete the upload state machine: `UploadStatus` and
  `db/queries_extra/cloud_attachments.rs`'s OUTGOING writers
  (`insert_cloud_attachment` / `update_cloud_attachment_status` / session /
  bytes-uploaded). CORRECTION (R2 finding H): there are NO "upload-only
  parts of `CloudProvider`" to delete. `CloudProvider`
  (`core/src/cloud_attachments.rs:37`) is a single enum -
  `OneDrive | GoogleDrive | Dropbox | Box` - that classifies BOTH outgoing
  hosting AND incoming detected links; the incoming carve-out
  (`detect_cloud_links`, § 2.6) uses the SAME enum. So `CloudProvider` is
  KEPT whole (the incoming carve-out needs it); only the outgoing
  upload-state types and writers are deleted.
- Table disposition decided NOW, not at land (R2 finding H). The
  `cloud_attachments` table cannot be dropped, because
  `insert_incoming_cloud_links_sync`
  (`db/queries_extra/cloud_attachments.rs:152`) writes `'incoming'` rows
  into it and is part of the retained § 2.6 carve-out. B9b therefore KEEPS
  the table and its incoming writer, and deletes only the outgoing writers /
  outgoing-status columns (or leaves the unused outgoing columns in place if
  a v100 column-drop is not worth the churn). This supersedes § 2.7's
  "pinned per sub-spec at land" deferral for the table decision.
- KEEP (carve-out, § 2.6): `detect_cloud_links`, the `enrich_*_link` fns,
  `CloudProvider`, `CloudLink`, and `insert_incoming_cloud_links_sync`,
  adjusted only to compile without the deleted upload types.
- `create_provider` and `ProviderOps::fetch_attachment` are NOT deleted
  here (other callers / B15); B9b removes no INCOMING surface.

## 5. Stopping rule (clause 9)

- IN: the two attachment seams. INCOMING fetch (`attachment.fetch` +
  prefetch) rewired onto `open_blob` / `open_raw_rfc822`; OUTGOING cloud
  hosting surface pinned onto `host_attachment` + hand-rolled duplicate
  deleted; the `blob_id` column; the B9-SQ engine passthrough.
- OUT, named not deferred:
  - `ProviderOps::fetch_attachment` trait + four impls + `create_provider`
    + `FetchedAttachment`: retire at B15 with the provider crates. B9
    removes the attachment CALLERS only.
  - The compose-side cloud-upload UX (threshold check, warn-vs-host UI,
    body link insertion, upload progress): NOT wired today (§ 2.5); a
    future item wires it against the § 4.2 surface. Not a B9 hole - B9
    ships the surface, and the feature was never present to preserve.
  - Incoming cloud-link detection / enrichment (`detect_cloud_links` /
    `enrich_*_link`): carved out (§ 2.6); no bifrost equivalent, currently
    unwired. B9 keeps them compiling, changes no behavior.
  - `open_blob_range` (partial fetch): the current fetch pulls whole
    blobs; B9 does not add range fetching (no caller needs it). The
    capability is there for a future streaming-viewer item.
- Blast radius: `service` (handler + prefetch + new module + boot wiring),
  `db` (one column + `AttachmentInsertRow`), `core` / `gmail` / `graph`
  (deletions), and the B9-SQ bifrost forwarder. No `app` / `service-api`
  wire change; no action-pipeline change; no sync-cursor change.

## 6. Verification per brick (clause 5)

Behavioral gates are mandatory (`docs/bifrost-migration.md` § 10): a
compile-only replacement of a live fetch path is under-gated. Each gate
below is the EXACT copy-pasteable command.

### 6.1 B9a gates (the live incoming path)

The per-provider attachment sync-harness scripts already exist
(`reference/glossary/harness.md`) and exercise real fetch against
`saehrimnir`; they are the primary gate and MUST stay green across the cut:

```
brokkr service-test crates/app/tests/sync-harness/jmap-attachment-initial.lua
brokkr service-test crates/app/tests/sync-harness/jmap-attachment-prefetch.lua
brokkr service-test crates/app/tests/sync-harness/jmap-attachment-cache-disabled.lua
brokkr service-test crates/app/tests/sync-harness/jmap-attachment-fetch-after-clear-cache.lua
brokkr service-test crates/app/tests/sync-harness/jmap-attachment-window-extend.lua
brokkr service-test crates/app/tests/sync-harness/jmap-rebuild-attachment-index-flag.lua
brokkr service-test crates/app/tests/sync-harness/gmail-attachment-initial.lua
brokkr service-test crates/app/tests/sync-harness/gmail-attachment-prefetch.lua
brokkr service-test crates/app/tests/sync-harness/graph-attachment-initial.lua
brokkr service-test crates/app/tests/sync-harness/graph-attachment-prefetch.lua
brokkr service-test crates/app/tests/sync-harness/imap-attachment-prefetch.lua
```

- The cache-disabled + fetch-after-clear-cache scripts force the
  CACHE-MISS branch (they drive the byte source, not just `materialize`),
  so they pin the `open_blob` / `open_raw_rfc822` rewire end-to-end.
- The `imap-attachment-prefetch.lua` gate pins the IMAP
  `open_raw_rfc822` + MIME-re-parse path. WARNING (R2 finding B): this
  script's message has a SINGLE attachment, so it CANNOT prove
  once-per-message. The load-bearing IMAP prefetch gate is
  `imap-folder-batch-session-reuse.lua`, which asserts 3 body fetches on
  1 connection (`:200-217`). A naive per-item `open_raw_rfc822` rewire
  BREAKS that gate (3 full-message downloads instead of 3 batched part
  fetches). B9a must explicitly RECONCILE or REPLACE that gate against the
  chosen IMAP batching design (§ 4.1.2), and add a multi-attachment IMAP
  fetch gate that proves the message is hydrated once. Reconciling this
  gate is a brick, laid with the rewire.
- Graph reference-attachment gate (R2 finding D): a new script that syncs a
  Graph message carrying a `#microsoft.graph.referenceAttachment` and
  asserts `attachment.fetch` on it surfaces the non-byte-stream error
  (NOT a cached empty file). Building it is a brick, laid before the rewire.
- Harness fixture obstacle (R2 finding G): the service-suite script
  `real_fixture_cache_miss_roundtrip.lua` seeds attachment bytes into the
  legacy `HarnessOfflineProvider` registry (`:65`, `test_helpers.rs:1097`).
  The new engine byte source cannot attach a `harness-offline` account, so
  the cache-miss it drives will not fetch through `open_blob`. The promised
  green `service-suite` requires a replacement fixture mechanism (route the
  offline fixture bytes through a `saehrimnir`-backed account, or teach the
  byte source a harness-offline shim). Resolving it is a brick, laid before
  the cut.
- Error-classification gate (R2 finding C): a unit/harness assertion that a
  provider timeout on the byte source maps to `SkipReason::ProviderTimeout`
  (feeding the circuit breaker) and an internal precondition maps to
  `SkipReason::InternalError`, proving `AttachmentByteError` preserves the
  distinctions `ServiceError` would have flattened.
- JMAP stale-handle gate (CONDITIONAL - R2 finding E): this gate exists
  ONLY if § 2.4's option (a) is chosen (persist a discriminator + keep the
  re-mint fallback). It requires a `saehrimnir` blob-rotation affordance
  that does NOT exist today, so building that affordance is a prerequisite
  brick. If § 2.4's default option (b) is taken (drop the re-mint fallback),
  this gate is NOT added and an expired-blob fetch simply surfaces the
  provider error, matching the legacy direct-download behavior.
- Deterministic unit gate for the handle reconstruction:

```
brokkr test -p service attachment_byte_source_reconstructs_blob_handle
```

  asserts `AttachmentByteSource` builds a `BlobHandle` whose `id` equals
  the verbatim persisted `blob_id` (not the unwrapped
  `remote_attachment_id`) for a Graph/Gmail row, and routes an IMAP row to
  `open_raw_rfc822`.
- The full suite as the green-tree backstop:

```
brokkr service-suite
```

- The prefetch performance budget (whole-message-once for IMAP; no
  request-count regression for HTTP): the existing attachment-prefetch
  sync-bench baseline, held against `brokkr.toml`:

```
brokkr sync-bench <attachment-prefetch-script> --gate <recorded-name>
```

  (R2 finding B: this command still contains `<placeholder>`s, violating
  clause 5's exact-copy-pasteable-gate requirement. Before the cut, the
  concrete script and `--gate <recorded-name>` MUST be substituted - if no
  attachment-prefetch sync-bench baseline exists yet, recording one and
  writing its literal name here is a brick laid before the rewire, not a
  land-time fill-in.)

## 9. Review reconciliation (R1 Opus + R2 codex xhigh)

Both B9-spec reviews were validated finding-by-finding against the frozen
`cf024ab` bifrost tree and the current ratatoskr source. Every valid
finding is folded into the sections above; this section is the index
(finding -> where folded -> code evidence) plus the disposition of the two
findings whose framing was narrowed. Severity labels are the reviewers'.

### 9.1 Accepted and folded

- **A [BUG, R1] IMAP "open_blob yields nothing" is factually false.**
  Bifrost IMAP `open_blob` is a real single-part `BODY.PEEK[section]` fetch
  (`research/bifrost/crates/imap/src/account/blob.rs:15-139`, decoded via
  `decode_blob_id`). The IMAP-routes-to-`open_raw_rfc822` decision is
  re-grounded on ratatoskr's own persistence (consumer stores only
  `part_id`, `hydrate.rs:684-699`), not a bifrost gap. Folded: § 1 (IMAP
  bullet), § 2.3, § 4.1.2, and the alternative (persist the IMAP handle) in
  finding B below.
- **B [BUG/HIGH, R1+R2] IMAP once-per-message not delivered; single-fetch
  regression.** Per-item `open_raw_rfc822` = N full-message downloads for an
  N-attachment message; today's session-reuse does 1 SELECT + N `BODY[part]`
  on 1 connection (`imap-folder-batch-session-reuse.lua:200-217`,
  `prefetch.rs:1011`). Folded: § 2.3 (CAUTION), § 4.1.2 (IMAP dispatch
  batching requirement), § 6.1 (gate reconciliation + placeholder removal).
  Two resolutions offered: message-level batch cache, or persist the IMAP
  `BlobHandle` and route IMAP to `open_blob` too.
- **C [HIGH, R2] `ServiceError` return loses prefetch error classes.**
  Prefetch needs `SkipReason::{ProviderTimeout, ProviderTransient,
  ProviderPermanent, InternalError}` + circuit breaker
  (`prefetch.rs:190-214`); `error_map.rs:5-25` returns `OperationResult` /
  `ActionError`, not `ServiceError`. Folded: § 4.1.2 (typed
  `AttachmentByteError`, per-caller mapping), § 6.1 (error-classification
  gate).
- **D [HIGH, R2] Graph reference attachments cached as empty files.** Graph
  `open_blob` yields `Warning(BlobNotByteStream)` + `Done` with no batch
  (`graph/src/account/blob.rs:121-125, 151-155`). Folded: § 4.1.2 (Graph
  reference-attachment guard - detect Warning, distinct error variant),
  § 6.1 (Graph reference gate).
- **E [HIGH, R2 + R1] JMAP stale-handle fallback is unsound.** Wrong API
  (`message_hydrate` is one `ObjectId` + `HydrationProjection`,
  `engine.rs:1436`; no `Projection::Metadata-with-blobs`), no stable
  discriminator to match the replacement (`Vec<BlobHandle>` only), and false
  parity claim (legacy JMAP does a direct `download(attachment_id)`,
  `jmap/src/ops.rs:535`, no re-derive). The NULL-`blob_id` trigger (§ 4.1.1)
  also makes it effectively all-provider, not JMAP-only (R1). Folded: § 2.4
  (rework-required, default: drop the fallback), § 4.1.2, § 6.1 (conditional
  gate).
- **F [HIGH, R2 + R1 nit] Outgoing host artifact cannot meet its attach /
  capability / error contracts.** `&SyncEngine` alone cannot attach a cold
  account; `account_id.into()` unsupported by `AccountId`; `ServiceError`
  has no unsupported variant; two-layer error shape collapsed. Folded:
  § 4.2.1 (four sketch corrections + the "pin ONLY the passthrough" fallback
  disposition).
- **G [HIGH, R2 + R1 nit] Survey misses breaking call sites.** Second
  `AttachmentInsertRow` construction at
  `provider-sync/src/graph/sync/persistence.rs:349`; in-test `CREATE TABLE`
  + `INSERT` at `message_persistence.rs:115-135, 153-164`;
  `real_fixture_cache_miss_roundtrip.lua` seeds the legacy
  `HarnessOfflineProvider` the engine path cannot attach. Folded: § 4.1.1
  (three-site edit), § 6.1 (fixture obstacle brick).
- **H [MEDIUM, R2] Cloud deletion plan internally contradictory.**
  `CloudProvider` (`cloud_attachments.rs:37`) has no upload-only variants -
  it classifies incoming links too; `insert_incoming_cloud_links_sync`
  (`db/.../cloud_attachments.rs:152`) writes `'incoming'` rows into the same
  table. Folded: § 2.7 + § 4.2.2 (keep `CloudProvider` + the table + the
  incoming writer whole; delete only outgoing writers/columns; decided now,
  not at land).
- **I [SMELL, R1] Verbatim blob id already embedded.** `attachments.id =
  "{message_id}_{blob.id.0}"` (`hydrate.rs:594`). Folded: § 2.4 (acknowledge
  recoverable-but-ambiguous; new column still chosen because `message_id`
  can contain `_`).
- **J [NIT, R1+R2] Landing-order contradiction.** Folded: § 3 (pinned:
  B9a -> B9-SQ -> B9b).

### 9.2 Narrowed (accepted in substance, framing tightened)

- **R2 finding B, "provider differences must not leak into ratatoskr as
  provider branches" (`bifrost-migration.md:45`).** The performance and
  gate-reconciliation core is ACCEPTED (folded as finding B). The framing
  that the HTTP-vs-IMAP branch is a forbidden provider-leak is NARROWED: the
  branch is forced by ratatoskr's own storage choice (IMAP rows persist
  `part_id`, HTTP rows persist a blob handle), not by a gratuitous provider
  `match`. It is a real design tension worth a bifrost-side uniform
  attachment surface eventually, but it is not, at `cf024ab`, a rule
  violation B9 must fix - so it is recorded as a tension and an alternative
  (§ 4.1.2), not mandated.

### 9.3 Rejected

- None. Every substantive claim in both reviews validated against the code.
  The only adjustments are the § 9.2 framing narrowing and the recognition
  that several R1/R2 items overlap (R1's single-fetch regression and R2's
  once-per-message finding are the same IMAP issue, folded together as B;
  R1's ordering nit and R2 finding 8 are the same, folded as J).

### 6.2 B9b gates (the outgoing surface + deletion)

The deleted code had no caller, so the gate is that nothing regresses and
the pinned surface compiles + dispatches capability-correctly:

```
brokkr check
brokkr test -p service host_attachment_dispatches_by_capability
```

The unit test asserts `host_large_attachment` forwards a
Gmail/Graph account to the engine passthrough and returns the
capability-absent error shape for a JMAP/IMAP account (the
`Unsupported(HostAttachment)` mapping), WITHOUT a per-provider `match` in
ratatoskr. The B9-SQ passthrough itself is gated bifrost-side by that
repo's `brokkr check` before promotion (§ 3).

### 6.3 The account-delete + cache invariants (regression guards)

B9 changes the byte source but not the delete/tombstone/eviction
lifecycle; the guards that protect it must stay green:

```
brokkr service-test crates/app/tests/sync-harness/jmap-account-delete-shared-blob.lua
```

(the account-is-deleting short-circuit at `handlers/attachment.rs:83-101`
still fires; the shared-blob delete path is unaffected by the source
swap.)

## 7. Stance (clause: structural over micro)

This is a plumbing replacement of the attachment byte source and the
deletion of a dead hand-rolled duplicate of a bifrost surface, labeled as
such. The incoming rewire is a source swap under an unchanged cache
pipeline; its risk is the BlobHandle reconstruction (resolved by
persisting the verbatim id, § 2.4) and the account-attach requirement
(resolved by reusing B4a's resident handle, § 4.1.2). The outgoing work is
overwhelmingly deletion: the § 1 maximal-integration rule mandates that
`gdrive.rs` / `onedrive.rs` cannot survive alongside A6 `host_attachment`,
and the survey (§ 2.5) shows they carry no live behavior to preserve.
Cleanliness is a deliverable - no env-var scaffolding, no dead upload
state machine left as "the way forward"; the single pinned outgoing
artifact (§ 4.2) is the built road the future compose item consumes.

## 8. Open items reconciled into the spec (no deferral holes)

- The BlobHandle reconstruction obstacle: resolved by a `blob_id` column +
  consumer persist (§ 2.4), gated by
  `attachment_byte_source_reconstructs_blob_handle` (§ 6.1). NOTE (R2
  finding E): the JMAP stale-handle `message_hydrate` re-mint fallback that
  once accompanied this is REWORK-REQUIRED and defaults to being DROPPED
  (§ 2.4) - wrong API, no stable discriminator, and it added behavior the
  legacy direct-download path never had. This item is resolved by the column
  alone; the fallback is not a load-bearing part of the resolution.
- The IMAP no-server-blob reality: resolved by routing IMAP to
  `open_raw_rfc822` + MIME re-parse (the consumer's own path), NOT
  `open_blob` (§ 2.3, § 4.1.2). Gated by `imap-attachment-prefetch.lua`.
- The account-attach requirement of `open_blob` / `open_raw_rfc822` /
  `host_attachment` (`live_account` -> `AccountNotAttached`): resolved by
  reaching the resident slot and attaching on demand, mirroring B4a's
  `ResidentActionAccount` (§ 4.1.2). Not a hole.
- The dead outgoing cloud subsystem (pivotal survey finding, § 2.5): B9
  DELETES the hand-rolled duplicate (§ 1 maximal-integration) and PINS
  `host_attachment` (§ 4.2) rather than newly-wiring a UX (feature-
  preserving, § 1). The compose-side UX is named OUT (§ 5), not deferred
  as a B9 hole - the feature was never present to preserve, and the
  surface it needs is shipped.
- Incoming cloud-link detection / enrichment (§ 2.6): carved out - no
  bifrost equivalent, currently unwired. Named, not deferred; kept
  compiling, behavior unchanged. The enrichment's direct-API wart is
  recorded for a future item that wires the incoming-link feature.
- The missing `SyncEngine::host_attachment` passthrough: resolved by B9-SQ
  (§ 3), the one bifrost side-quest, mirroring the existing passthrough
  clusters; advances the freeze from `cf024ab`, recorded in § 11 at land.
  B9a needs no bifrost change and is independently landable.
- `ProviderOps::fetch_attachment` / `create_provider` / `FetchedAttachment`
  survival: named, excluded to B15 (§ 2.2, § 5). B9 removes the attachment
  callers only; the trait and factory keep compiling for their other
  callers. Not a deferral hole.
- The `cloud_attachments` table disposition (§ 2.7): the outgoing columns
  retire with the dead machine; the additive-green table-vs-drop choice is
  pinned per sub-spec at land, leaving no orphaned writer.
- Wire-contract stability: `AttachmentFetchParams` / `AttachmentFetchAck`
  and the `core`/`app` firewall are unchanged; every bifrost type B9
  touches stays inside `service` (§ 2.3). Verified, not assumed.
