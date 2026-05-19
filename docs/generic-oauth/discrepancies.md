# Generic OAuth: Spec vs. Code Discrepancies

Audit date: 2026-05-19 (previous: 2026-03-30). Item #7 (WebFinger) shipped this pass; remaining items confirmed against current code.

---

## Critical

1. ~~**Re-auth broken for generic/OIDC providers.**~~ ✅ Fixed - re-auth detects the `oidc:` prefix on the stored provider id and re-discovers endpoints at runtime via `probe_issuer()` instead of failing on registry lookup. See `crates/app/src/ui/add_account/oauth.rs:44-89` (the `oidc:` strip + `probe_issuer` call inside `start_reauth_oauth`).

## Missing features

2. **No manual issuer URL flow in wizard.** The spec's main UX proposal is "enter issuer URL, discover, then authenticate." The manual setup UI offers exactly four providers: `Gmail`, `Microsoft365`, `Jmap`, `Imap` (see `ManualProvider::ALL` driving the provider selector in `crates/app/src/ui/add_account/manual_config.rs:101-120`). There is no "Custom OIDC" provider entry, and no input field for an issuer URL anywhere in the wizard. `probe_issuer()` exists and works (`crates/core/src/discovery/oidc.rs:110`) but is only reachable from re-auth, never from add-account.

3. **No manual client ID / client secret entry.** Client ID resolution is hardcoded in `resolve_client_id()` at `crates/app/src/ui/add_account/oauth.rs:245-252`: Microsoft default for Microsoft, empty string otherwise. Client secret is hardcoded to `None` at every `OauthCaptureConfig` construction site (`oauth.rs:104`, `state.rs:653`). On-prem providers that require a registered (confidential) client cannot be configured through the UI.

4. **JMAP OAuth manual config explicitly unsupported.** The wizard hardcodes the error string `"JMAP OAuth is not yet supported for manual configuration. Please use password authentication."` at `crates/app/src/ui/add_account/manual_config.rs:60-62`, then forces the user back to the manual-config step. Blocks generic OIDC against JMAP servers.

5. **SMTP OAuth path is XOAUTH2-only.** `crates/smtp/src/client.rs:27-32` forces `vec![Mechanism::Xoauth2]` for any `auth_method == "oauth2"` config. There is no `OAUTHBEARER` mechanism wired into the SMTP transport, so a generic OIDC provider whose SMTP submission accepts only OAUTHBEARER (RFC 7628) cannot submit mail. IMAP got OAUTHBEARER (RFC 7628) via the implementation noted below; SMTP did not.

6. **No SASL mechanism auto-detect from CAPABILITY.** `authenticate()` in `crates/imap/src/connection.rs:344-391` selects between `OAUTHBEARER`, `XOAUTH2`, and `LOGIN` purely from `config.auth_method`; the IMAP CAPABILITY response is never consulted for `AUTH=` mechanism advertisement. CAPABILITY *is* fetched after authentication (`connection.rs:411-417`) but only for CONDSTORE/QRESYNC. The caller must specify the mechanism explicitly.

## Nice-to-have (deferred)

8. **Dynamic client registration (RFC 7591)** - No `registration_endpoint` handling; `grep -r registration_endpoint` returns no hits. Low priority since most on-prem providers require pre-registered clients anyway.

9. **Custom scope entry** - No UI surface for IT admins or users to specify additional provider-specific scopes beyond the negotiated defaults; the scopes come from the registry entry or from the OIDC discovery document's `scopes_supported`, with no override mechanism.

10. **IT-distributable config file** - No `ratatoskr-config.json` / TOML / similar for pre-seeding provider configuration (issuer + mail servers + client ID); the only mention of "admin config" in the codebase is an unrelated `Display name from Autodiscover or admin config` comment at `crates/app/src/db/types.rs:33`.

## Resolved since spec was written

- **OIDC discovery** - `.well-known/openid-configuration` fetch, scope negotiation, PKCE S256 detection, public-client detection. Implemented in `crates/core/src/discovery/oidc.rs`; entry point `probe_issuer()` at line 110.
- **OIDC cascade integration** - Parallel probe and `OAuth2Unsupported` → `OAuth2` upgrade with domain-relationship check. `crates/core/src/discovery/mod.rs:97-134`.
- **OAUTHBEARER authenticator (RFC 7628)** - IMAP path implemented at `crates/imap/src/connection.rs:68-75` (the `OAuthBearer` SASL struct) and wired into authentication at `connection.rs:349-365`.
- **Token refresh** - Provider-agnostic, handles rotation, supports public clients (no secret). `crates/common/src/token.rs:43-84` (`refresh_oauth_token`).
- **WebFinger (RFC 7033)** - Email-based OIDC issuer discovery for email domains that delegate to a different IdP host (e.g. `corp.com` → `auth.corp.com`). `crates/core/src/discovery/webfinger.rs` queries `https://{domain}/.well-known/webfinger?resource=acct:user@domain&rel=http://openid.net/specs/connect/1.0/issuer`, validates the returned `href`, then feeds it into `oidc::probe_issuer`. Wired in `crates/core/src/discovery/mod.rs` as a sequential precursor to the OIDC stage (WebFinger first, bare-domain probe as fallback).

## Notes for the next pass

- Items #2 and #3 are the load-bearing UX gaps and are tightly coupled - there is no useful "issuer URL only" flow without a client-id input alongside it. Treat them as one piece of work.
- Item #5 has gotten relatively more important since the original audit: now that IMAP OAUTHBEARER works, a generic OIDC provider can land in a state where IMAP auth succeeds but SMTP submission silently fails for users whose server requires OAUTHBEARER on submission. Worth elevating from "missing feature" to "consistency bug" framing.
- Item #6 is small and self-contained backend work; would be a sensible single-PR change.
