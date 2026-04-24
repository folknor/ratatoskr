# Generic OAuth: Spec vs. Code Discrepancies

Audit date: 2026-03-30

---

## Critical

1. ~~**Re-auth broken for generic/OIDC providers.**~~ ✅ Fixed - re-auth now detects `oidc:` prefixed providers and discovers endpoints at runtime via `probe_issuer()` instead of failing on registry lookup.

## Missing features

2. **No manual issuer URL flow in wizard.** The spec's main UX proposal is "enter issuer URL, discover, then authenticate." The manual setup UI only offers Gmail, Microsoft365, JMAP, and IMAP - no custom OIDC provider option.

3. **No manual client ID / client secret entry.** Client ID resolution is hardcoded to Microsoft default and empty string otherwise. On-prem providers that require a registered client cannot be configured.

4. **JMAP OAuth manual config explicitly unsupported.** The wizard shows "JMAP OAuth is not yet supported for manual configuration." Blocks generic OIDC for JMAP providers.

5. **SMTP OAuth for generic providers unresolved.** The spec raises this as an open question. No completed generic SMTP OAuth submission flow exists - unclear whether generic OIDC tokens are passed through to SMTP auth.

6. **SASL mechanism auto-detect from CAPABILITY.** IMAP CAPABILITY is checked for CONDSTORE/QRESYNC but not for `AUTH=OAUTHBEARER` vs `AUTH=XOAUTH2`. The caller must specify the auth method explicitly. Should auto-negotiate.

## Nice-to-have (deferred)

7. **WebFinger (RFC 7033)** - Email-based issuer discovery. Not implemented. Low priority since the OIDC cascade already upgrades email-domain discovery results.

8. **Dynamic client registration (RFC 7591)** - No `registration_endpoint` handling. Low priority since most providers require pre-registered clients.

9. **Custom scope entry** - No mechanism for IT admins to specify additional provider-specific scopes beyond the negotiated defaults.

10. **IT-distributable config file** - No `ratatoskr-config.json` or similar for pre-seeding provider configuration.

## Resolved since spec was written

- OIDC discovery (`.well-known/openid-configuration` fetch, scope negotiation, PKCE S256 detection, public client detection)
- OIDC cascade integration (parallel probe, `OAuth2Unsupported` → `OAuth2` upgrade with domain-relationship check)
- OAUTHBEARER authenticator (RFC 7628) - implemented in `crates/imap/src/connection.rs`
- Token refresh - provider-agnostic, handles rotation
