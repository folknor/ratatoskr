# Generic OAuth: Spec vs. Code Discrepancies

Audit date: 2026-05-19 (previous: 2026-03-30). Items #2 / #3 / #4 (Custom OIDC wizard), #7 (WebFinger), #8 (dynamic registration), #9 (custom scopes plumbing) all shipped; #6 in flight blocked on an async-imap fork. Remaining: #5 SMTP OAUTHBEARER (blocked on lettre), #10 IT-distributable config file (deferred).

---

## Critical

1. ~~**Re-auth broken for generic/OIDC providers.**~~ ✅ Fixed - re-auth detects the `oidc:` prefix on the stored provider id and re-discovers endpoints at runtime via `probe_issuer()` instead of failing on registry lookup. See `crates/app/src/ui/add_account/oauth.rs:44-89` (the `oidc:` strip + `probe_issuer` call inside `start_reauth_oauth`).

## Missing features

5. **SMTP OAuth path is XOAUTH2-only.** `crates/smtp/src/client.rs:27-32` forces `vec![Mechanism::Xoauth2]` for any `auth_method == "oauth2"` config. There is no `OAUTHBEARER` mechanism wired into the SMTP transport, so a generic OIDC provider whose SMTP submission accepts only OAUTHBEARER (RFC 7628) cannot submit mail. IMAP got OAUTHBEARER (RFC 7628) via the implementation noted below; SMTP did not. Blocked on lettre's `Mechanism` enum not exposing OAUTHBEARER - either patch/fork lettre or roll our own SMTP SASL.

6. **No SASL mechanism auto-detect from CAPABILITY.** `authenticate()` in `crates/imap/src/connection.rs:344-391` selects between `OAUTHBEARER`, `XOAUTH2`, and `LOGIN` purely from `config.auth_method`; the IMAP CAPABILITY response is never consulted for `AUTH=` mechanism advertisement. CAPABILITY *is* fetched after authentication (`connection.rs:411-417`) but only for CONDSTORE/QRESYNC. **In flight**: blocked on an async-imap fork that exposes pre-auth `Client::capabilities()` so the dispatch can read advertised mechanisms before attempting auth. Plan at `.plans/moonlit-herding-cookie.md` § Slice A.

## Nice-to-have (deferred)

10. **IT-distributable config file** - No `ratatoskr-config.json` / TOML / similar for pre-seeding provider configuration (issuer + mail servers + client ID); the only mention of "admin config" in the codebase is an unrelated `Display name from Autodiscover or admin config` comment at `crates/app/src/db/types.rs:33`.

## Resolved since spec was written

- **OIDC discovery** - `.well-known/openid-configuration` fetch, scope negotiation, PKCE S256 detection, public-client detection. Implemented in `crates/core/src/discovery/oidc.rs`; entry point `probe_issuer()` at line 110.
- **OIDC cascade integration** - Parallel probe and `OAuth2Unsupported` → `OAuth2` upgrade with domain-relationship check. `crates/core/src/discovery/mod.rs:97-134`.
- **OAUTHBEARER authenticator (RFC 7628)** - IMAP path implemented at `crates/imap/src/connection.rs:68-75` (the `OAuthBearer` SASL struct) and wired into authentication at `connection.rs:349-365`.
- **Token refresh** - Provider-agnostic, handles rotation, supports public clients (no secret). `crates/common/src/token.rs:43-84` (`refresh_oauth_token`).
- **WebFinger (RFC 7033)** - Email-based OIDC issuer discovery for email domains that delegate to a different IdP host (e.g. `corp.com` → `auth.corp.com`). `crates/core/src/discovery/webfinger.rs` queries `https://{domain}/.well-known/webfinger?resource=acct:user@domain&rel=http://openid.net/specs/connect/1.0/issuer`, validates the returned `href`, then feeds it into `oidc::probe_issuer`. Wired in `crates/core/src/discovery/mod.rs` as a sequential precursor to the OIDC stage (WebFinger first, bare-domain probe as fallback).
- **Custom scope override (plumbing)** - `accounts.oauth_extra_scopes TEXT NULL` column stores space-separated extras; `merge_scopes` in `crates/app/src/ui/add_account/oauth.rs` unions negotiated + extras (deduped, order preserved); re-auth reads the column and forwards merged scopes through `OauthCaptureConfig`. The wizard surface that would let a user type extras hasn't been built yet (separate from #2/#3); IT config file (#10) is the natural alternate source.
- **Dynamic client registration (RFC 7591)** - `OidcDiscoveryDocument` and `OidcEndpoints` now carry an optional `registration_endpoint` (validated as HTTPS at discovery time). New module `crates/core/src/discovery/dyn_registration.rs` provides `register()` (RFC 8252 public-client request shape, hardening matching `oidc.rs` / `webfinger.rs`). The OAuth flow in `start_reauth_oauth` invokes registration automatically when the user-supplied client ID is empty and the IdP advertises a registration endpoint; the resulting `client_id` (and `client_secret`, if any) flows through `OAuthSuccess` and is persisted on the account row.
- **Custom OIDC wizard provider (#2 / #3 / #4)** - Two new variants on `ManualProvider`: `CustomOidcImap` (OIDC + IMAP/SMTP servers) and `CustomOidcJmap` (OIDC + JMAP session URL). Wizard form collects issuer URL plus optional client ID/secret; submit constructs `provider_id = "oidc:{issuer}"`, validates inputs via `oidc::is_valid_https_url`, and feeds the existing `start_reauth_oauth` flow. The pre-existing `ManualProvider::Jmap` branch is now password-only (the JMAP-OAuth error string at `manual_config.rs:60-62` is removed); users wanting JMAP+OAuth pick `CustomOidcJmap`. New `oauth_client_secret` column in the create-account IPC and DB schema persists confidential-client secrets when registered or supplied manually.

## Notes for the next pass

- Item #5 remains the most painful gap: now that IMAP OAUTHBEARER works (and the Custom OIDC wizard surfaces generic providers), a deployment can land in a state where IMAP auth succeeds but SMTP submission silently fails for users whose server requires OAUTHBEARER on submission. Either patch lettre or roll our own SMTP SASL.
- Item #6 is in flight - waiting on the async-imap fork to land. Ratatoskr-side change is ~50 lines per the plan.
- Item #10 (IT-distributable config file) becomes more useful now that the wizard exists: an IT admin can ship a JSON/TOML that pre-seeds issuer URL + client ID + mail servers so end users don't fill the form by hand.
