# B14 technical-implementation-spec: account construction, discovery, verify

The onboarding-seam Track B item. Replaces ratatoskr's hand-rolled
account connection test with bifrost's `AccountFactory::open`, moving the
test off the app-side provider-crate call and onto the Service-side
bifrost factory. Discovery (the five-stage cascade) and OAuth
authorization (browser redirect plus code exchange) are KEPT verbatim;
only the connection-test / verify surface is rewired, and the now-dead
`ProviderOps::test_connection` / `get_profile` surface is deleted.

**Feature-preserving mandate (governing principle,
`docs/bifrost-migration.md` § 1).** B14 is a plumbing replacement of ONE
capability: "prove these credentials reach a live account before we
commit the account row." The user-visible onboarding flow (discover ->
pick protocol / enter credentials or run OAuth -> credentials verified ->
Identity step -> account created) is preserved. What changes underneath
is the mechanism of the verify step and where it runs, not the flow.

This spec is written against
`reference/technical-implementation-spec.md` (the contract it must
satisfy - READ IT) and conforms to its ten clauses. It is one item of
`docs/bifrost-migration.md` (the governing plan and TODO source - READ
§ 3, § 4, § 5, § 7 B1/B14, § 8, § 9, § 11), run through
`reference/orchestrate.md`.

## Required reading (clause 10)

Every implementer and reviewer MUST read these before laying a brick.
They are the ground this work is built on and judged against; naming
them is not enough.

- `reference/technical-implementation-spec.md` - the contract this spec
  is written against. The clause numbers below are its clauses.
- `reference/architecture.md` - ALWAYS required. The load-bearing rule
  for B14 is the core/UI firewall: the app depends on `rtsk` plus
  `service-api` wire types only, and **bifrost must never become a
  dependency of `core` (`rtsk`)** (`docs/bifrost-migration.md` § 7 B1
  done-note). B14's whole point is that the new verify path cannot live
  in `core` the way `verify_imap` does today; it moves Service-side.
  The `MailActionIntent -> resolve_intent -> build_execution_plan ->
  batch_execute` pipeline and the `OperationResult` taxonomy are NOT on
  B14's path (verify is an onboarding IPC, not an action), but the wire
  contract discipline is.
- `UI.md` (repo root) - REQUIRED: B14 rewires wizard state
  (`AddAccountStep::Validating`, `handle_submit_credentials`,
  `handle_oauth_success`, the `ValidationComplete` transition), which is
  UI work under the project rule "any UI work - read UI.md". R2 finding
  11 flagged its omission from this list.
- `docs/bifrost-migration.md` - the TODO source. § 5 (Inventory: the
  four provider crates and `common`'s `ProviderOps` retire; verify's
  provider-crate use is part of that surface), § 7 B1 done-note (what
  `build_account_factory` / `DbWriteBackTokenSource` already deliver -
  B14 builds directly on them), § 7 B14 (this item), § 9 (risks), § 11
  (the frozen `../bifrost` commit discipline).
- `reference/glossary/harness.md` - the Service test harness, sync-harness
  scripts, `brokkr service-test` / `service-suite` / `sync-bench`,
  `saehrimnir` mock servers, and gate baselines. The verify gates this
  spec pins are defined there; the new `account.verify` request must be
  reachable from a Lua script the same way `TestSeedAccount` /
  `TestQueryDbState` are.
- `research/bifrost/reference/sync.md` § Construction / § Lifecycle - the
  `AccountFactory` surface. `SyncEngine::attach` step 2 is
  `factory.open(account_id).await -> Arc<dyn Account>`; B14 calls that
  same `open` directly (without an engine) as the connection test, then
  `Account::close()`. Read the `attach` / `detach` / `reopen`
  description so the verify path's open-then-close matches the engine's
  own lifecycle contract.
- `research/bifrost/reference/{imap,jmap,google,graph}.md` - the four
  `AccountFactory::open` implementations, which is what makes `open` a
  REAL connection test rather than a cheap constructor. Verified at the
  frozen pin: IMAP `open` opens a connection pool, authenticates
  (`authenticate_best`), runs `ID` / QRESYNC / folder `LIST`
  (`imap.md` "Configuration and connect" / account layer); Gmail `open`
  does one `users.getProfile` round-trip and parses the history id
  (`google.md`); JMAP `open` connects a `Client`, resolves the primary
  `Mail` account, reads the session, and probes initial state
  (`jmap.md`); Graph `open` builds the client and reads capabilities.
  Each authenticates and touches the network, so a bad password /
  revoked token / unreachable host surfaces as an `AccountError` out of
  `open`. THIS is the connection test.
- `research/bifrost/reference/error-model.md` - `AccountError`,
  `AccountErrorKind`, `RecoveryClass`, and the message-key namespace.
  `open`'s failure is an `AccountError`; B14 maps it to a user-facing
  verify result through the existing `error_map.rs`.

The `../bifrost` dependency checkout is frozen for the full duration of
this item per `docs/bifrost-migration.md` § 11. Record the exact pin
(`git -C ../bifrost rev-parse HEAD`, run in the main conversation, not a
subagent) in the ground survey of the landing, and do not let
`../bifrost` mutate underneath the in-flight step. `../bifrost` (the
build dependency) and `./research/bifrost` (the reading reference cited
above) are the same tree at the same commit; every citation here is
pinned to that tree. At the time of this spec both trees are at
`8e1006e` (`8e1006e0a7efc7f87958cead6b17c9b39b66c9c9`); the landing's
ground survey re-runs `git -C ../bifrost rev-parse HEAD` and records the
pin it actually built against.

## 1. The goal (clause 7: the target as concrete artifacts)

Today the account connection test is a per-onboarding-path patchwork,
and one leg of it lives in the wrong crate:

- **Password / IMAP accounts.** The app calls
  `rtsk::account::verify_imap::verify_imap_credentials`
  (`crates/core/src/account/verify_imap.rs:11`) DIRECTLY, in-process
  (the app depends on `rtsk`), from
  `crates/app/src/ui/add_account/password_auth.rs:225` (the
  `AddAccountStep::Validating` step). That function builds an
  `imap::types::ImapConfig` and calls `imap::connection::connect`
  (`verify_imap.rs:20/30`) - i.e. `core` depends on the `imap` provider
  crate (`crates/core/Cargo.toml:39`) purely to run this one test.
  SMTP is not verified at all on this path.
- **OAuth accounts (Gmail / Graph / JMAP).** There is NO explicit
  connection test. Reachability is proven only implicitly by
  `oauth.exchange_code`'s token-endpoint round-trip plus userinfo fetch
  (`crates/service/src/handlers/oauth.rs:73`,
  `rtsk::oauth::exchange_code_with_provider`). The tokens are then shipped
  to `account.create` unverified against the actual mail endpoint.
- **A dead trait surface.** `ProviderOps::test_connection` and
  `ProviderOps::get_profile` (`crates/common/src/ops.rs:162/166`),
  their `ProviderTestResult` / `ProviderProfile` types
  (`crates/common/src/types.rs:118-132`), and the per-provider impls
  (`crates/{imap,gmail,graph,jmap}/src/ops.rs`, plus the offline harness
  impl at `crates/service/src/actions/provider.rs:246`) exist but have
  **zero production callers** (verified: no `.test_connection(` /
  `.get_profile(` call site anywhere outside the provider crates' own
  internals). This is the abandoned first draft of a unified connection
  test.

B14 collapses all three into one Service-side test built on bifrost's
`AccountFactory::open` (`docs/bifrost-migration.md` § 4 shift 4 -
capability dispatch: ratatoskr calls one uniform surface, bifrost picks
the per-provider primitive). After B14:

- A new Service-side verify path builds a bifrost `Arc<dyn
  AccountFactory>` from the IN-FLIGHT onboarding credentials (NOT a
  persisted account row - the account does not exist yet), calls
  `factory.open(account_id).await`, and on success immediately calls
  `Account::close()`. `open` succeeding IS the connection test: it
  authenticated and touched the provider (IMAP LOGIN + LIST, Gmail
  getProfile, JMAP session probe, Graph capabilities). `open` failing
  yields an `AccountError` mapped to a user-facing verify error.
- The app's `AddAccountStep::Validating` step calls this over a new
  `account.verify` IPC (`ServiceClient`), NOT the in-process
  `rtsk::account::verify_imap`. Both onboarding legs route through it:
  the password/IMAP leg ships host/port/security/username/password; the
  OAuth leg ships the freshly-exchanged tokens plus the token/endpoint
  metadata. The verify step gates progression to the Identity step
  exactly as the password leg does today.
- `core` drops its `imap` provider-crate dependency for verify:
  `crates/core/src/account/verify_imap.rs` is deleted, `mod verify_imap`
  is removed from `crates/core/src/account/mod.rs`, and the `imap = {
  path = "../imap" }` line leaves `crates/core/Cargo.toml` (confirmed
  sole `imap::` use in `core` is `verify_imap.rs`; the `graph::client`
  use in `cloud_attachments.rs:9` is B9's surface and the `pub use smtp`
  re-export at `lib.rs:38` are OUT of scope - § 6).
- The dead `ProviderOps::test_connection` / `get_profile` surface, the
  `ProviderTestResult` / `ProviderProfile` types, and every per-provider
  impl are deleted. (The rest of `ProviderOps` - the action methods -
  survives to B4/B15 per `docs/bifrost-migration.md` § 7 B15.)

Discovery is UNTOUCHED (`crates/core/src/discovery/` - the five-stage
cascade: registry, autoconfig, MX, jmap-wellknown, port-probe, plus the
parallel WebFinger / OIDC lane). OAuth AUTHORIZATION is UNTOUCHED
(`rtsk::oauth`, the redirect + `oauth.exchange_code` code exchange +
userinfo). B14 changes what happens with the discovered config and the
exchanged tokens AFTER those steps: they feed `AccountFactory::open` as
the connection test instead of `imap::connection::connect` or nothing.

The target seam, pinned to concrete types:

```
app add-account wizard (AddAccountStep::Validating)
  -> ServiceClient::verify_account(VerifyAccountParams)   [NEW IPC]
  -> service handler crates/service/src/handlers/account_verify.rs
       1. VerifyAccountParams -> DecryptedAccountCredentials-shaped
          in-flight input (NO DB row read; NO persistence)
       2. factory = factory_from_decrypted(in_flight, provider, writer)   [refactor of build_account_factory's match arms]
       3. account = factory.open(synthetic_account_id).await   <-- THE CONNECTION TEST
       4. account.close().await                                <-- release immediately; nothing persisted
       5. Ok  |  Err(AccountError) -> error_map -> VerifyAccountAck { ok: false, message }
  -> AddAccountMessage::ValidationComplete(gen, Result<(), String>)   [unchanged app-side variant]
       Ok  -> AddAccountStep::Identity (unchanged)
       Err -> re-show credential screen with the mapped message (unchanged)
```

`factory.open` is the same call `SyncEngine::attach` makes internally
(`research/bifrost/reference/sync.md` § Lifecycle step 2); B14 uses it
standalone as a pre-persist reachability probe and discards the handle
(`close()`), never attaching it to the engine. The engine attach that
drives real sync happens later, after `account.create`, on the existing
B3 path - B14 does not touch it.

## 2. Survey of the ground (clause 8)

### 2.1 What B1 already laid that B14 consumes

- `crates/service/src/bifrost/factory.rs::build_account_factory`
  (`:96`) returns `Arc<dyn AccountFactory>` for Gmail / Graph / JMAP /
  IMAP. Today it is DB-row-driven: it reads
  `read_bifrost_account_credentials(conn, account_id)` (`:674`),
  `MailProviderKind::parse`es the row's provider, `row.decrypt(key)`s
  into `DecryptedAccountCredentials` (`:477`), and matches on provider
  to build the per-provider factory (`:110-173`). B14 refactors the
  per-provider MATCH (`:110-173`, plus the `build_jmap_factory` `:250`
  and `build_imap_factory` `:276` helpers) so it is reachable from BOTH
  the row-read path (unchanged callers) AND a new in-flight-credentials
  path. The match already takes `(&DecryptedAccountCredentials or
  DecryptedAccountCredentials, MailProviderKind, WriterPool)` - the
  refactor lifts it into `factory_from_decrypted(...)` and leaves
  `build_account_factory` as `read row -> decrypt ->
  factory_from_decrypted`.
- `DecryptedAccountCredentials` (`factory.rs:477`) with `is_oauth`
  (`:491`), `username` (`:495`), `oauth_token_source` (`:541`),
  `required_plain` / `required_secret` / `optional_port`, and the
  wrapped `AccountCredentialsRow` (`:386`). The in-flight verify input
  must produce this SAME struct so `factory_from_decrypted` is provider-
  agnostic to whether the credentials came from a row or from the wire.
- `crates/service/src/bifrost/token_source.rs::DbWriteBackTokenSource`
  (A1) - the generic `Arc<dyn TokenSource>` OAuth refresher. For verify,
  the freshly-exchanged access token is used directly; a write-back
  refresher is unnecessary for a one-shot open (the token was minted
  seconds earlier). The in-flight path can build a static-token source
  or reuse the from-access-token factory constructors that the harness
  arms already use (`from_access_token_with_api_base` `:118`,
  `GraphClient::with_api_bases` `:160`) - see § 4.2 for the exact
  choice.
- `crates/service/src/bifrost/error_map.rs::account_error_to_action_error`
  / `account_error_to_operation_result` (`:5/:10`) - maps `AccountError`
  -> `ActionError` / `OperationResult` via `RecoveryClass` +
  `message_key()`. B14 reuses this to turn an `open` failure into the
  verify result's user-facing message.
- `BifrostBuildError` (`factory.rs:49` region) - the factory-construction
  error (unknown provider, missing/undecryptable credential, missing
  endpoint, invalid config). Verify surfaces these too (a malformed
  in-flight config fails before `open` is even reached).

### 2.2 The current verify surfaces B14 rips out

- `crates/core/src/account/verify_imap.rs` (whole file, 33 LOC):
  `verify_imap_credentials`, the app's in-process IMAP connection test.
  Its ONLY caller is
  `crates/app/src/ui/add_account/password_auth.rs:225`
  (`validate_imap_connection` -> `AddAccountMessage::ValidationComplete`).
- `crates/core/src/account/mod.rs:3` - `pub mod verify_imap;` line.
- `crates/core/Cargo.toml:39` - `imap = { path = "../imap" }`, plus the
  `imap` mentions in the `hotpath` / `hotpath-alloc` feature lists
  (`:69/:70`). Confirmed the `imap` crate is used in `core` ONLY by
  `verify_imap.rs` (the `async-imap` dep at `Cargo.toml:43` is a
  separate direct dependency - audit at implementation whether removing
  `verify_imap` orphans it; if it has no other `core` user, drop it too,
  otherwise leave it and note why).
- `crates/common/src/ops.rs:160-166` - the `ProviderOps` "Connection /
  Profile" section: `test_connection` + `get_profile` trait methods.
- `crates/common/src/types.rs:118-132` - `ProviderTestResult` /
  `ProviderProfile` structs (and the `test_connection` / `get_profile`
  mention in the `ProviderCtx` doc comment at `:35`).
- The per-provider impls: `crates/imap/src/ops.rs:1015`,
  `crates/graph/src/ops/mod.rs:334`, `crates/gmail/src/ops.rs:238`,
  `crates/jmap/src/ops.rs:544`, and the offline-harness impl at
  `crates/service/src/actions/provider.rs:246`. Deleting the trait
  methods forces these to go; each provider's INTERNAL helpers
  (`imap_client::test_connection` at `crates/imap/src/client/mod.rs:559`,
  `smtp::client::test_connection` at `crates/smtp/src/client.rs:182`)
  are separate - the IMAP/SMTP crate-internal `test_connection` helpers
  are NOT `ProviderOps` and are B15's disposition, not B14's (they are
  unreachable once the trait method that called them is gone, but
  deleting the whole provider crate is B15). B14 removes ONLY the
  `ProviderOps` methods and their per-provider trait impls; if a
  provider's `test_connection` trait-impl body was the sole caller of
  its crate-internal helper, the helper becomes dead but is left for
  B15's crate deletion (do not chase it - § 6 stopping rule).

Note (R-style survey correction): because `test_connection` /
`get_profile` have NO production callers, deleting them is behavior-
neutral - no wire type, notification, or UI surface observes them. This
is a dead-code excision bundled with the live rewire, not a
behavior-preserving cutover of its own.

### 2.3 The onboarding wire flow B14 rewires

The two create entry points converge on
`service::accounts::create_account_inner`
(`crates/service/src/accounts/create.rs:25`), which is a thin
`with_write(create_account_sync)` plus a documented (currently no-op)
post-create hook. B14 does NOT fold verify into `create_account_inner`
(see § 3 for why): verify must gate the Identity step, which happens
BEFORE `account.create` in the wizard, so a create-then-verify-then-
rollback shape would both reorder the UX and risk a persisted-but-
unreachable row on a rollback failure. Verify is a distinct pre-create
IPC.

- Password/IMAP leg: `password_auth.rs::handle_submit_credentials`
  (`:16`) sets `AddAccountStep::Validating`, spawns
  `validate_imap_connection` (`:211`), and on `ValidationComplete(Ok)`
  advances to `AddAccountStep::Identity`
  (`state.rs:619/655-656`); on `Err` re-shows `PasswordAuth`
  (`state.rs:660-663`). Re-auth mode (`reauth_account_id.is_some()`)
  short-circuits to `account.update_tokens` (`state.rs:622-652`) - that
  branch must keep verifying too (§ 4.3).
- OAuth leg: `oauth.exchange_code` (`handlers/oauth.rs:40`) returns
  tokens + email + display_name to the UI (`OauthExchangeCodeAck`);
  `handle_oauth_success` (`state.rs:668`) runs the Identity step, then
  `account.create` ships the tokens. Today there is no explicit endpoint
  reachability test between exchange and create. B14 inserts an
  `account.verify` call on this leg too (§ 4.3), so a token that
  exchanged fine but cannot actually open the mailbox (wrong scope,
  provider-side block) is caught before the row is written.

### 2.4 What discovery/OAuth-authorization already own (KEPT, surveyed to bound the rip)

- `crates/core/src/discovery/` - `discover(email)` (`mod.rs:22`) runs the
  five-stage cascade (`run_stages` `:394`: registry / autoconfig / MX /
  jmap-wellknown; `run_probe_stage` `:456`: port probe) plus the parallel
  WebFinger + OIDC lane (`:54`) and the OAuth2Unsupported -> OAuth2 OIDC
  upgrade (`:126`). B14 touches NONE of this. The discovered
  `DiscoveredConfig` still drives protocol selection and pre-fills the
  credential screen exactly as today.
- `rtsk::oauth` - `exchange_code_with_provider`, `GenericOAuthProvider`,
  the userinfo fetch. KEPT. `oauth.exchange_code` still runs the token
  round-trip; B14 only adds a verify step consuming its output.

## 3. The split (clause 6: keep/revert, ordered so the tree stays green)

B14 is a single item but has an ordering constraint: the dead-surface
deletion (§ 2.2 `ProviderOps` methods) and the live rewire (§ 2.1-2.3)
are independent, and the live rewire must be fully wired and gated
before the app-side `verify_imap` call is removed (the app cannot lose
its verify path for even one commit). The tree is green at every
boundary.

Two ordered landings:

### B14a - the bifrost verify path (the live rewire)

The single intrusive landing. It:

1. Refactors `factory.rs`: extract `factory_from_decrypted(decrypted,
   provider, writer) -> Result<Arc<dyn AccountFactory>, BifrostBuildError>`
   from `build_account_factory`'s match (§ 2.1). `build_account_factory`
   and `build_calendar_account_factory` keep their exact signatures and
   behavior (row read -> decrypt -> `factory_from_decrypted`).
2. Adds the in-flight credential constructor (§ 4.2):
   `VerifyAccountParams -> DecryptedAccountCredentials` without a DB row.
3. Adds the `account.verify` wire type + handler (§ 4.1, § 4.4) that
   builds the factory, calls `factory.open(synthetic_id)`, `close()`s,
   and maps the outcome.
4. Rewires the app: `AddAccountStep::Validating` calls
   `ServiceClient::verify_account` instead of
   `rtsk::account::verify_imap`, on BOTH legs (§ 4.3). Deletes
   `crates/core/src/account/verify_imap.rs`, the `mod verify_imap` line,
   and the `imap` dep from `crates/core/Cargo.toml`.

At B14a's boundary the tree is green: `brokkr check` passes and the new
verify gates (§ 5) pass. Both onboarding legs verify through bifrost.
The dead `ProviderOps::test_connection` / `get_profile` surface still
compiles (untouched here) but is now provably unreachable.

### B14b - dead-surface deletion

A separate follow-on landing on a green B14a. Deletes
`ProviderOps::test_connection` / `get_profile`, `ProviderTestResult` /
`ProviderProfile`, and the five per-provider impls (§ 2.2). Pure
excision; regresses nothing because the methods had no callers. Bundled
here rather than in B14a to keep the live rewire's diff reviewable and
its revert clean.

Order: B14a, then B14b. B14a delivers the working bifrost verify
(green, shippable). B14b removes the corpse. Neither regresses the
other. (An implementer may land both as one commit if `brokkr check`
stays green and the diff is reviewable; the ordering exists to keep the
revert boundary clean, not to force two commits.)

## 4. The bricks

### 4.1 The `account.verify` wire type (concrete artifact)

New request in `crates/service-api/src/` (alongside the account request
types; confirm the exact module - `request.rs` enum arm plus a params
struct, mirroring `AccountCreateParams` / `OauthExchangeCodeParams`).

```rust
/// Pre-persist connection test. Carries the same in-flight credential
/// shape the create flow is about to persist, so `AccountFactory::open`
/// can prove reachability before `account.create` writes the row.
/// Secret fields use `RedactedString` (same redaction as
/// OauthExchangeCodeParams / AccountUpdateTokensParams).
pub struct VerifyAccountParams {
    pub provider: String,          // "gmail_api" | "graph" | "jmap" | "imap"
    pub email: String,
    // IMAP / password leg:
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub imap_security: Option<String>,   // "tls" | "starttls" | "none"
    pub username: Option<String>,
    pub imap_password: Option<RedactedString>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_security: Option<String>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<RedactedString>,
    pub accept_invalid_certs: bool,
    // OAuth leg:
    pub access_token: Option<RedactedString>,
    pub jmap_url: Option<String>,        // JMAP session URL when known
    pub caldav_url: Option<String>,      // DAV compose (mirrors the row column)
}

pub struct VerifyAccountAck {
    pub ok: bool,
    pub message: Option<String>,   // user-facing failure text when !ok
}
```

`account.verify` joins `RequestParams::bypasses_admission()` (the same
admission-bypass list as `oauth.exchange_code` / `health.ping` /
`boot.ready`, `handlers/oauth.rs:11-13`) so the connection test is not
queued behind heavy traffic.

**CORRECTION (R2 finding 5): the budget must exceed bifrost's own open
timeouts, not merely match `oauth.exchange_code`.** `AccountFactory::
open` for IMAP runs authenticate + `ID` + QRESYNC + `LIST`
*sequentially*, and bifrost's IMAP layer alone carries a 30 s connect
timeout plus a 60 s command timeout
(`research/bifrost/crates/imap/src/connection/config.rs:31`). Reusing
OAuth's ~30 s budget would let the IPC time out before `open` does,
bypassing the promised mapped `VerifyAccountAck`. Pin an explicit total
verify deadline that strictly exceeds the worst-case bifrost open
(connect + the sequential command timeouts) - or set a verify-specific
factory timeout - so a slow/unreachable host surfaces as a mapped verify
failure rather than a transport timeout. State the exact value in the
landing.

The params intentionally mirror the field set
`read_bifrost_account_credentials` produces on the row, so § 4.2's
constructor is a straight field map, not a translation.

### 4.2 In-flight `DecryptedAccountCredentials` (the pre-persist credential source)

The engine/sync path builds the factory from a persisted, encrypted row
(`read_bifrost_account_credentials` -> `row.decrypt(key)`). Verify has no
row and no ciphertext - the UI holds plaintext credentials it is about to
ship to `account.create`. So B14 builds `DecryptedAccountCredentials`
DIRECTLY from `VerifyAccountParams`, bypassing both the DB read and the
decrypt step:

- Construct an `AccountCredentialsRow` (`factory.rs:386`) with a
  SYNTHETIC id (a fresh `uuid`, never persisted), `email`, `provider`,
  and the transport fields (`imap_host` / `imap_port` / `imap_security`
  / `jmap_url` / `caldav_url` / `accept_invalid_certs`) from the params.
- Populate the plaintext credential fields (`access_token`,
  `imap_password`, etc.) that `DecryptedAccountCredentials` exposes
  post-decrypt directly from the params' redacted fields
  (`RedactedString::into_inner`).
- Set `row.auth_method` explicitly. **CORRECTION (R1 finding 1 / R2
  finding 4):** `is_oauth()` (`factory.rs:491`) does NOT classify from
  token presence - it reads `matches!(self.row.auth_method.as_str(),
  "oauth2" | "oauth" | "bearer")`. `VerifyAccountParams` (§ 4.1) carries
  no `auth_method`, so the constructor MUST derive and set it (write
  `"oauth2"` when `access_token` is present, else `"password"`) or the
  wire type must add an explicit `auth_method` field. Left unset, every
  verify gets an empty `auth_method`, `is_oauth()` returns `false` for
  all providers, and OAuth verify wrongly builds a password factory
  (JMAP -> `JmapCredentials::Basic` `factory.rs:264`; IMAP -> the
  password branch) and fails.
- Populate `encryption_key: [u8; 32]` (`factory.rs:487`). The struct
  requires it even though the verify path never decrypts. On the static-
  token path it is also never consumed (write-back is disabled, § 4.2
  OAuth note), so a zero-filled key is acceptable ONLY because it is
  provably unused here; do not route it into any live cipher call.
- `is_oauth()` then classifies from the `auth_method` set above, and
  `factory_from_decrypted` dispatches per `MailProviderKind::parse(&row.
  provider)` exactly as the row path does.

The exact constructor shape depends on `DecryptedAccountCredentials`'s
field privacy (`factory.rs:477-540`). Two options, pick at
implementation and state the choice in the landing commit:
(a) add an associated constructor `DecryptedAccountCredentials::from_
plaintext(row, plaintext_fields)` beside the existing `row.decrypt`,
reused by nothing else today but the honest home for "credentials that
did not come from an encrypted row"; or (b) if the struct's fields are
already crate-visible, build it inline in the verify handler. (a) is
preferred (keeps the invariant "how you get a `DecryptedAccountCredentials`"
in one file) unless it forces widening visibility that (b) avoids.

**OAuth token source for verify.** `factory_from_decrypted`'s OAuth arms
call `decrypted.oauth_token_source(provider, writer)` to build a
`DbWriteBackTokenSource`. For a NON-persisted verify account there is no
row to write back to, so a refreshing/write-back source is wrong. The
verify path uses the freshly-minted access token directly via the
factory constructors that already take a bare access token (the harness
arms use exactly these):
`GoogleAccountFactory::from_access_token_with_api_base` (`factory.rs:118`),
`GraphClient::with_api_bases(.., access_token)` (`:160`),
`JmapCredentials::Bearer { token_source }` where the source is a static
single-token source. **CORRECTION (R2 finding 4):** `bifrost_net::
StaticTokenSource` ALREADY exists at the frozen pin (`8e1006e`; used by
`research/bifrost/crates/imap/src/types/auth.rs:65`,
`graph/src/client.rs:59`), constructed `StaticTokenSource::new(token,
None)`. No in-tree helper is needed - pin this type directly rather than
leaving it to implementation (Risk § 7 "static token source
availability" is thereby resolved, not open). Wire this by giving
`factory_from_decrypted` (or a thin verify-only sibling) a `TokenMode {
WriteBack(writer) | Static(token) }` parameter so the row path keeps
write-back and the verify path uses the static token.

**CORRECTION (R1 finding 2): the `TokenMode` refactor spans all four
match arms, not just Gmail/Graph.** Both `build_jmap_factory`'s Bearer
arm (`factory.rs:261`) and `build_imap_factory`'s OAuth branch
independently call `decrypted.oauth_token_source(...)`, which hard-
requires `refresh_token` AND `oauth_client_id` via `required_plain`
(`factory.rs:544-546`) plus a resolvable token endpoint - none of which
verify carries. So `TokenMode::Static` must be threaded through the JMAP
and IMAP helpers too; naming only the Gmail/Graph/JMAP constructors
understates the surface. Keep the per-provider construction otherwise
identical - the goal is that verify exercises the SAME open path sync
will, differing only in token lifetime.

### 4.3 The app rewire (concrete artifact)

`crates/app/src/ui/add_account/password_auth.rs`:

**CORRECTION (R2 finding 3): verify must test the SAME provider
`account.create` will persist - do not hardcode `provider = "imap"`.**
`account.create` persists `self.resolved_provider` (`identity.rs:133`),
which the manual-config step can set to `gmail_api` / `graph`
(`manual_config.rs:75`) or `oidc:{issuer}` (`manual_config.rs:143`, which
`MailProviderKind::parse` rejects). A verify that hardcodes `"imap"`
could pass against an IMAP endpoint while create then writes a
Gmail/Graph row, or the OAuth leg could ship an unparseable
`oidc:{issuer}`. `VerifyAccountParams.provider` MUST carry the same
resolved mail-protocol / OAuth-provider identity the wizard will persist
(`self.resolved_provider`), resolved ONCE and shared by verify and
create, so the two never diverge.

- `validate_imap_connection` (`:211-235`) is deleted. `handle_submit_
  credentials` (`:16`) instead builds `VerifyAccountParams` from
  `self.auth_state` (host/port/security/username/password/smtp/certs,
  `provider = self.resolved_provider` per the correction above) and
  dispatches
  `ServiceClient::verify_account(params)` in the `Task::perform`,
  mapping to the UNCHANGED `AddAccountMessage::ValidationComplete(gen,
  Result<(), String>)` (map `VerifyAccountAck { ok: true }` -> `Ok(())`,
  `{ ok: false, message }` -> `Err(message)`, transport error ->
  `Err(...)`). The `ValidationComplete` handling in `state.rs:616-664`
  is UNCHANGED - it already advances to Identity on `Ok` and re-shows
  the credential screen on `Err`.
- The `service_client` handle is already on the wizard
  (`state.rs:377-381`); the password leg simply uses it now (today it
  bypasses Service and calls `rtsk` directly).

OAuth leg (`handle_oauth_success`, `state.rs:668`): after
`oauth.exchange_code` succeeds, insert an `account.verify` call (build
`VerifyAccountParams` with `provider` = the resolved OAuth provider,
`access_token` = the returned token, `jmap_url` when JMAP) BEFORE
advancing to Identity. A verify failure here surfaces the mapped message
and holds the user at the OAuth/protocol screen rather than creating an
unreachable account.

**CORRECTION (R2 finding 2): the OAuth leg CANNOT route through the
existing `ValidationComplete` handler unchanged.** As written
(`state.rs:616-664`), the `Ok(())` branch, when `reauth_account_id` is
set, treats every success as a PASSWORD re-auth - it ships
`AccountUpdateTokensParams` with `imap_password` and no access token
(`state.rs:627-640`) - and the `Err` branch ALWAYS forces `step =
AddAccountStep::PasswordAuth` (`state.rs:662`), contradicting the
promise of returning the OAuth user to the OAuth/protocol screen. So the
OAuth verify result needs an origin-tagged variant or a separate
message/transition (e.g. an `OAuthValidationComplete(gen, Result)` whose
`Err` re-shows the OAuth/protocol step and whose `Ok` advances to
Identity without the password-reauth branch). Reusing `ValidationComplete`
verbatim is not viable; § 4.3's earlier "prefer reusing" guidance is
superseded by this correction.

Re-auth leg (`state.rs:622-652`): re-auth is meant to verify the NEW
credentials before persisting them. **CORRECTION (R2 finding 1): for the
OAuth re-auth path, verify-before-persist is not achievable with a
post-exchange `account.verify` as specified.** `oauth.exchange_code`,
when `reauth_account_id` is set, IMMEDIATELY persists the new tokens
(`oauth.rs:117-122` `update_account_tokens_sync`), detaches + reattaches
the resident account (`oauth.rs:125-135`), and returns an ack with
`access_token: None` (`oauth.rs:140`). The UI therefore never receives
the new token to hand to `account.verify`, and the old credential is
already overwritten. B14 must EITHER (a) make the re-auth exchange
return the unpersisted tokens (deferring the write until after verify),
OR (b) perform the verify INSIDE the exchange handler, before it commits
the new tokens. The password re-auth path (which carries the plaintext
password in `self.auth_state` and persists via `account.update_tokens`,
not inside exchange) can still `account.verify` the in-flight
credentials before `update_tokens`. State the chosen OAuth-re-auth shape
in the landing; the "MAY verify from the row post-update" option is
rejected - it verifies AFTER persistence, defeating the pre-persist
guarantee.

### 4.4 The verify handler (concrete artifact)

New `crates/service/src/handlers/account_verify.rs`:

```rust
pub(crate) async fn handle_verify_account(
    boot_state: &Arc<BootSharedState>,
    params: Box<VerifyAccountParams>,
) -> Result<Value, ServiceError> {
    let writer = boot_state.writer_pool()?;             // for the (unused-on-verify) token plumbing shape
    let decrypted = decrypted_from_verify_params(*params)?;   // § 4.2, no DB, no decrypt
    let provider = MailProviderKind::parse(&decrypted.row.provider)
        .map_err(|_| /* BifrostBuildError::UnknownProvider -> VerifyAccountAck{ok:false} */)?;
    let synthetic_id: AccountId = /* fresh uuid */;
    let ack = match factory_from_decrypted_static(decrypted, provider, writer) {
        Err(build_err) => VerifyAccountAck { ok: false, message: Some(build_err.to_string()) },
        Ok(factory) => match factory.open(synthetic_id).await {
            Ok(account) => {
                let _ = account.close().await;          // release immediately; nothing persisted
                VerifyAccountAck { ok: true, message: None }
            }
            Err(account_error) => {
                // reuse error_map: AccountError -> user-facing message
                let action_error = crate::bifrost::error_map::account_error_to_action_error(&account_error);
                VerifyAccountAck { ok: false, message: Some(user_message(action_error)) }
            }
        },
    };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}
```

Key invariants:

- **Nothing is persisted.** No `create_account_sync`, no cursor, no
  checkpoint, no engine attach. The synthetic id is discarded. A failed
  or succeeded verify leaves the DB untouched; `account.create` (later,
  separate IPC) is the only writer.
- **`open` is the test.** The handler does not need to CALL any account
  method beyond `open` - `open` already authenticated and hit the
  network (§ 1, § required-reading). Immediately `close()`.
- **Failure mapping.** `BifrostBuildError` (malformed in-flight config -
  e.g. missing host, bad security mode) and `AccountError` (auth
  failed, host unreachable, TLS refused) both become
  `VerifyAccountAck { ok: false, message }`. Register
  `handle_verify_account` in `crates/service/src/handlers/mod.rs`
  dispatch and add `account.verify` to the admission-bypass list.
- **CORRECTION (R2 finding 9): the pseudocode's `BifrostBuildError` arm
  contradicts the no-leak rule and has no sanitizer.** The § 4.4 sketch
  returns `build_err.to_string()`, but `error_map.rs` only maps
  `AccountError` -> message key (`error_map.rs:5`); there is NO
  `BifrostBuildError` sanitizer. `BifrostBuildError` carries raw internal
  detail (account ids, decrypt-failure strings, config detail), so
  `to_string()` leaks it. Specify an explicit `BifrostBuildError` ->
  stable-message mapping (a small match producing user-safe text per
  variant, e.g. "Missing or invalid connection setting"), and route the
  `AccountError` arm through `error_map` as shown.
- **CORRECTION (R1 finding 4): the mapped text is message-key-grade, not
  polished prose.** `account_error_to_action_error` sets `message =
  err.message_key()` (`error_map.rs:7`); `ActionError::user_message()`
  (`actions.rs:209`, self-documented as still incorporating internal
  wording) wraps it as e.g. `"Server rejected: auth.invalid-credentials"`.
  So a bad-password verify shows a namespaced key with a category prefix,
  not "Incorrect password." This is acceptable for B14 (matches the
  existing action surface and is no worse than today's raw connect
  error), but the spec should not overstate it as clean user-facing
  copy. Call `action_error.user_message()` explicitly where the § 4.4
  sketch writes `user_message(action_error)`.

**Harness reachability.** The gate scripts (§ 5) drive verify from Lua.
`account.verify` is a normal wire request, so a Lua script can call
`client:request("account.verify", { ... })` (or a `TestVerifyAccount`
shim if the wire request needs an app-only shape) directly, the same way
`imap-login-multi-account.lua` calls `TestSeedAccount`. Because verify
does `factory.open`, saehrimnir must answer the open handshake for the
tested provider - which it already does for IMAP (LOGIN + LIST, proven
by `imap-login-multi-account.lua:99-101`) and for the OAuth providers
(the discovery / oauth harness scripts and the per-provider sync
fixtures exercise the same open path).

## 5. Verification per brick (clause 5)

Per gate, the EXACT command. `brokkr check` is the universal green-tree
gate for every landing.

**Universal:**

```
brokkr check
```

**Brick: `factory_from_decrypted` refactor + in-flight constructor
(§ 2.1, § 4.2).** The existing `factory.rs` test module already asserts
"each `MailProviderKind` dispatches to a working factory arm"
(`factory.rs:808`, `seed_oauth(.., "gmail", "gmail_api", "google", ..)`
at `:823`). Extend it with an in-flight-credentials case per provider
that builds `DecryptedAccountCredentials` from a synthetic
`VerifyAccountParams` (no DB). **CORRECTION (R2 finding 8): the test
CANNOT assert which concrete factory type came back.** `Arc<dyn
AccountFactory>` has no `Any`/downcast hook, no `Debug`, and no provider
tag at the frozen pin - the existing test documents exactly this
(`factory.rs:808-819`). Prove dispatch the way that test already does:
assert `Ok` for each provider's kind-specific, non-interchangeable
credential shape (JMAP needs `jmap_url`; IMAP needs host + password; the
OAuth kinds need a token + `auth_method`) and assert the right
`BifrostBuildError` for a malformed config (missing host, missing
`jmap_url`, unset `auth_method` forcing a wrong arm). A row routed to the
wrong arm fails a required-column read rather than returning `Ok`.
Deterministic, in-process, no network.

```
brokkr test -p service factory_from_decrypted
```

(Confirm the package name for the crate owning `factory.rs` - it is the
`service` crate per `crates/service/src/bifrost/factory.rs`. Adjust the
`-p` value and the substring to the actual test names.)

**Brick: the verify handler + wire type (§ 4.1, § 4.4).** A deterministic
unit test on `decrypted_from_verify_params` (params -> correct
`is_oauth()` classification and transport fields, per provider) and on
the `AccountError -> VerifyAccountAck.message` mapping (feed a
constructed `AccountError` of an auth-failure kind, assert `ok == false`
and a non-empty message; feed a build error, same). Model the
`AccountError` construction on `error_map.rs`'s own tests
(`error_map.rs:243+` builds `AccountErrorBuilder` cases).

```
brokkr test -p service verify_account
```

**Brick: end-to-end verify over the mock provider (§ 4.3, § 4.4) - the
load-bearing gate.** New sync-harness scripts under
`crates/app/tests/sync-harness/`, run against saehrimnir (the doc for
this harness is `reference/glossary/harness.md` - READ IT):

- `account-verify-imap-success.lua` - spawn, boot.ready, call
  `account.verify` with valid IMAP credentials pointing at the
  saehrimnir IMAP endpoint (reuse the `multi-account-small.toml` fixture
  and the LOGIN-by-username wiring `imap-login-multi-account.lua`
  relies on), assert `ok == true`, assert (via
  `harness.mock_requests`) that a LOGIN + LIST reached the mock (proving
  `open` really ran the handshake), and assert NO account row was
  persisted (a follow-up `TestQueryDbState` / account-list shows the
  verify did not create the account).
- `account-verify-imap-bad-password.lua` - same, wrong password, assert
  `ok == false` and a non-empty `message`, and again assert no row
  persisted. **CORRECTION (R2 finding 6): this gate cannot pass against
  saehrimnir as it stands.** saehrimnir's IMAP `LOGIN` accepts EVERY
  credential and uses only the username for account selection
  (`research/saehrimnir/src/imap.rs:414-434`; saehrimnir's "auth is
  opt-in, always-accept" baseline). A wrong password therefore still
  yields `ok == true`. This gate has a PREREQUISITE brick: either add an
  auth-rejection capability to saehrimnir (e.g. a fixture flag or an
  `on("imap","LOGIN", ...)` override returning `NO`, using the Lua
  dispatch surface saehrimnir already exposes), OR drive a different
  deterministic `open` failure instead (unreachable host / port, or a
  malformed-config `BifrostBuildError`) and rename the script
  accordingly. Pick one and land the saehrimnir change (if any) first;
  do not write a gate that cannot fail.
- `account-verify-oauth.lua` - verify a Gmail (or JMAP) OAuth account by
  shipping a token the saehrimnir mock accepts; assert `ok == true` and
  that the provider's open round-trip (Gmail `users.getProfile` / JMAP
  session) hit the mock. **CORRECTION (R1 finding 3): this gate does NOT
  exercise the new static-token construction.** Under the harness env
  vars (`RATATOSKR_TEST_GMAIL_ENDPOINT` / `RATATOSKR_TEST_GRAPH_ENDPOINT`)
  the Gmail/Graph arms ALREADY bypass `oauth_token_source` and use the
  bare-token constructors (`factory.rs:112-160`), so the harness takes a
  pre-existing branch and the production `TokenMode::Static` path (JMAP
  Bearer + the non-test-endpoint OAuth arms) is covered only by the unit
  test. Add a unit test that drives `factory_from_decrypted` with
  `TokenMode::Static` and NO test-endpoint env var set (or over the JMAP
  Bearer arm, which has no bare-token bypass), or explicitly record this
  gate's blind spot.

**CORRECTION (R2 finding 7): the e2e plan proves the IPC, not the wired
wizard, and misses several paths.** Every gate above calls
`account.verify` directly, so BOTH wizard legs could remain unwired and
still pass. The plan additionally omits: (a) OAuth verify-rejection; (b)
the OAuth re-auth ordering (R2 finding 1); (c) the four distinct static-
token factory arms; (d) a `close()`/LOGOUT assertion - the IMAP success
gate checks LOGIN + LIST but not LOGOUT, so dropping `close()` still
passes (add a LOGOUT/`close` assertion, or assert the connection was
released). Also, driving `account.verify` from Lua REQUIRES a harness
registry entry (`reference/glossary/harness.md:135`) - that registration
is itself a brick and must be listed, not assumed. Cover the wizard
wiring either with an app-level test that exercises `handle_submit_
credentials` / `handle_oauth_success` end to end, or state explicitly
that wiring is verified by manual/inspection and why.

```
brokkr service-test crates/app/tests/sync-harness/account-verify-imap-success.lua
brokkr service-test crates/app/tests/sync-harness/account-verify-imap-bad-password.lua
brokkr service-test crates/app/tests/sync-harness/account-verify-oauth.lua
```

(Confirm whether these belong to the Service-harness or sync-harness
suite and adjust the `brokkr service-test` vs the sync-harness runner
accordingly per `reference/glossary/harness.md`; the credential/open
round-trip against saehrimnir places them with the sync-harness family
alongside `imap-login-multi-account.lua`.)

**Brick: dead-surface deletion (B14b, § 2.2).** No behavior to pin (no
callers). The gate is `brokkr check` staying green after the deletion
(the compiler proves every impl and type reference is gone). No new test
is owed; state this explicitly per clause 5.

**Performance gate (clause 5 / clause 10).** Verify is on the
interactive onboarding path, not a sync/provider hot path or a batch
loop, and it does exactly one `open` + `close` per user action. It
carries no provider-request-count, elapsed, or peak-RSS budget, so NO
`brokkr sync-bench` baseline is owed. This is the explicit clause-5
statement that the behavior is not perf-gated; `brokkr check` plus the
harness scripts stand in. (If verify were ever called in a loop, this
would change - it is not.)

## 6. Stopping rule (clause 9: bounded blast radius)

In scope: the connection-test / verify mechanism only. The rebuild stops
at:

- **Discovery is untouched.** `crates/core/src/discovery/` (all five
  stages plus WebFinger/OIDC) stays exactly as is. The five-stage
  cascade is explicitly KEPT (`docs/bifrost-migration.md` § 7 B14).
- **OAuth authorization is untouched.** `rtsk::oauth`,
  `oauth.exchange_code`, the redirect handler, the userinfo fetch - all
  KEPT. B14 consumes their output, adds no OAuth logic.
- **`account.create` / `create_account_inner` are untouched.** Verify is
  a separate pre-create IPC; the create path is not rewired (its
  post-create hook stays the documented no-op).
- **The `ProviderOps` ACTION methods survive.** Only `test_connection` /
  `get_profile` are deleted. The action methods retire at B4/B15
  (`docs/bifrost-migration.md` § 7 B15). Do not touch them.
- **Provider-crate deletion is B15.** B14 removes `core`'s `imap`
  dependency (its sole use was `verify_imap`) and the `ProviderOps`
  verify methods, but does NOT delete the `gmail` / `jmap` / `graph` /
  `imap` crates, `provider-sync`, or the crate-internal
  `imap_client::test_connection` / `smtp::client::test_connection`
  helpers - those retire in the final B15 collapse. A provider's
  `test_connection` crate-internal helper that becomes dead when its
  `ProviderOps` impl is deleted is LEFT for B15; do not chase orphaned
  helpers across the provider crates.
- **`core`'s other provider-crate touchpoints are out of scope.** The
  `graph::client::GraphClient` use in
  `crates/core/src/cloud_attachments.rs:9` is B9's surface; the `pub use
  smtp` re-export at `crates/core/src/lib.rs:38` and the `async-imap`
  direct dep both stay unless the `imap`-removal audit proves `async-imap`
  is orphaned (§ 2.2). B14 removes only what `verify_imap` pulled.
- **The engine attach path is untouched.** Verify does a standalone
  `open` + `close` and never attaches to `BifrostSyncEngine`. Real sync
  attach stays on the B3 path, post-create.
- **Verify is inbound-mailbox-only (R1-6 / R2-10).** `AccountFactory::
  open` authenticates the mail/IMAP endpoint but does NOT connect or
  authenticate SMTP submission
  (`research/bifrost/crates/imap/src/account/factory.rs:288`). B14 does
  not add an SMTP probe; adding one is a scope increase. The verify
  result must not be presented as proving SMTP credentials, and the
  `smtp_*` params are disposed per § 8 (drop, or keep only with
  documented create-parity intent).

## 7. Risks

- **Pre-persist token lifetime.** Verify uses the freshly-exchanged
  access token statically (no write-back source, § 4.2). If the token
  expired between exchange and verify (seconds), `open` fails with an
  auth error and the user retries - acceptable, and no worse than
  today's implicit exchange-only check. The mitigation is that verify
  runs immediately after exchange.
- **`open` cost on a bad host.** `AccountFactory::open` for IMAP opens a
  pool and negotiates TLS/QRESYNC; against an unreachable or slow host it
  can block up to bifrost's connect timeout. Verify inherits that
  timeout; the UI already shows a "Validating credentials..." spinner
  (`password_auth.rs:192`), so a slow verify degrades gracefully. Confirm
  the handler budget (§ 4.1) exceeds the bifrost connect timeout so the
  IPC does not time out before `open` does.
- **DAV compose on verify.** IMAP `open` fail-soft-opens CardDAV/CalDAV
  sub-accounts (`imap.md` account layer). A DAV outage degrades to
  IMAP-only and does NOT fail the open (`DavAttach::Degraded`), so verify
  correctly passes on a reachable mailbox with a flaky DAV endpoint -
  matching the engine's own posture. No special handling needed; just do
  not treat a DAV warning as a verify failure.
- **Static token source availability.** If `bifrost-types` at the frozen
  pin has no ready-made static `TokenSource`, § 4.2 adds a trivial one
  in `service`. Confirm before speccing the exact type; this is a small
  in-tree helper, not a bifrost change (bifrost is frozen - § 11).
- **Harness open reachability.** The verify gates depend on saehrimnir
  answering `open` for each tested provider. IMAP LOGIN+LIST is proven
  reachable (`imap-login-multi-account.lua`); confirm the Gmail/JMAP
  open round-trips the OAuth gate needs are mounted before writing
  `account-verify-oauth.lua`, and scope that script to whichever
  provider saehrimnir answers most completely if one lags.

## 8. Review-correction ledger (R1 + R2 consolidated)

Both review reports (`B14-R1.md`, Opus; `B14-R2.md`, Codex) were
validated finding-by-finding against the tree at pin `8e1006e`. Every
finding was confirmed against the cited code; none was rejected on
validity grounds. The load-bearing corrections are folded inline at the
sections above; this ledger is the index plus the remaining minor
findings.

**Folded inline above:**

- R1-1 / R2-4 (`is_oauth()` reads `row.auth_method`, not token presence;
  struct needs `encryption_key`; `StaticTokenSource` exists at pin) -> § 4.2.
- R1-2 (`TokenMode` refactor spans JMAP + IMAP arms too, via
  `oauth_token_source`) -> § 4.2.
- R1-3 (harness OAuth arms bypass `oauth_token_source`, so the e2e OAuth
  gate is blind to the static-token path) -> § 5.
- R1-4 / R2-9 (message is message-key-grade; `build_err.to_string()`
  leaks and has no sanitizer) -> § 4.4.
- R2-1 (OAuth re-auth persists tokens inside `exchange_code` before
  verify is possible) -> § 4.3.
- R2-2 (`ValidationComplete` cannot be reused unchanged for OAuth) -> § 4.3.
- R2-3 (verify must use `resolved_provider`, not hardcoded `"imap"`) -> § 4.3.
- R2-5 (verify deadline must exceed bifrost's IMAP connect+command
  timeouts) -> § 4.1.
- R2-6 (bad-password gate cannot fail against always-accept saehrimnir) -> § 5.
- R2-7 (gates prove the IPC, not the wired wizard; missing close()/LOGOUT,
  OAuth-reject, re-auth, four-arm, and harness-registry coverage) -> § 5.
- R2-8 (cannot assert concrete factory type) -> § 5.
- R2-11, UI.md sub-point (missing required reading) -> Required reading;
  pin sub-point -> § intro (pin `8e1006e` now recorded).

**Remaining minor findings, folded here:**

- **R1-5 (dead `writer` binding).** § 4.4's pseudocode fetches `writer =
  boot_state.writer_pool()?` "for the token plumbing shape," but under
  `TokenMode::Static` the writer is never consumed - a dead binding /
  clippy smell. Do NOT fetch the writer pool on the verify path when the
  static token mode is selected; thread `TokenMode::Static(token)` in
  without a `WriterPool` at all.
- **R1-6 / R2-10 (SMTP is not verified; `smtp_*` params are dead).**
  bifrost IMAP `open` authenticates IMAP and constructs a
  `SubmissionTransport` but never connects or authenticates SMTP
  (`research/bifrost/crates/imap/src/account/factory.rs:288`). B14 is
  therefore explicitly an INBOUND-MAILBOX-ONLY reachability test; state
  this in § 1 and § 6 so the flow does not imply "all supplied
  credentials were tested." The five `smtp_*` fields in
  `VerifyAccountParams` (§ 4.1) are consequently unread by verify. Either
  drop them from the wire type, or keep them ONLY if create-parity /
  future SMTP-probe intent is documented at the field - do not carry
  silent dead wire surface. Adding a real SMTP probe (connect + AUTH) is
  a scope increase and is NOT in B14 unless the user asks.
- **R1-7 (IMAP-over-XOAUTH2 unrepresentable).** The deleted `verify_imap`
  took `auth_method` `"password" | "xoauth2"`; the new params default the
  password/IMAP leg to a password credential. With § 4.2's correction
  (`auth_method` derived from `access_token` presence), an IMAP account
  that authenticates via XOAUTH2 would need `access_token` populated on
  the IMAP leg - which § 4.1 currently scopes to the "OAuth leg" only.
  Confirm whether any onboarding path produces IMAP + XOAUTH2 (e.g.
  Gmail-via-IMAP); if it does, the IMAP leg must be allowed to carry
  `access_token` and set `auth_method = "oauth2"`. If no such leg exists,
  say so in § 1 so "collapses all three into one" is unambiguous.

**Partially rejected:**

- **R2-11, pin sub-claim** ("does not record the exact bifrost pin in its
  survey"). Partially rejected: the spec DELIBERATELY defers capturing
  the built-against pin to the landing's ground survey
  (`git -C ../bifrost rev-parse HEAD`, run in the main conversation),
  consistent with its own § 11 discipline - that is procedure, not an
  omission. Accepted in substance anyway: the current pin (`8e1006e`) is
  now stated directly in the intro so a reader is not left guessing,
  while the landing still re-runs and records the pin it actually built.
  The UI.md and "resolve the confirm/adjust/pick deferrals" sub-points of
  R2-11 are ACCEPTED and folded (UI.md into required reading;
  `StaticTokenSource`, the factory-type assertion, and the timeout are
  now pinned to concrete resolutions above rather than left open).
