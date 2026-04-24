# Generic OAuth2/OIDC for On-Prem Email

## The Problem

No desktop email client supports custom OAuth2/OIDC providers. Every client hardcodes flows for Google, Microsoft, and sometimes Apple - but if you're running Dovecot + Postfix behind Keycloak, Authentik, Authelia, or any other OIDC provider, you're stuck with app passwords.

This forces organizations into one of two bad choices:

1. **Move to cloud email** just for SSO/2FA support in clients, even though their mail servers already speak OAuth2.
2. **Disable 2FA for email** by issuing app passwords, undermining their entire security posture.

Almost every modern mail server supports OAuth2 authentication via SASL OAUTHBEARER/XOAUTH2. The server side is solved. The client side is the bottleneck.

## Why This Matters

On-prem email is not a niche. Universities, hospitals, government agencies, defense contractors, and privacy-conscious companies all run their own mail infrastructure. Many have invested heavily in SSO (Keycloak, Authentik, Okta, Azure AD on-prem, ADFS) and require 2FA across all services. Email is often the last holdout - the one service where users still type a password because their client can't do anything else.

Ratatoskr would be the first desktop email client to close this gap.

## What We Have Today

Our OAuth2 infrastructure is already provider-agnostic in the core plumbing:

- **`GenericOAuthProvider`** accepts configurable endpoints, scopes, PKCE, optional client secret
- **IMAP XOAUTH2 authenticator** is wired up and working
- **Token refresh** is provider-agnostic with per-account locking
- **DB schema** already stores per-account `oauth_token_url`, `oauth_client_id`, `oauth_client_secret`
- **`AuthMethod::OAuth2`** in discovery types carries all the right fields
- **Autoconfig XML parsing** already detects `authentication="oauth2"` but punts on unknown providers (`OAuth2Unsupported`)

The gap is narrow but critical: **there's no way to discover or manually configure a custom OIDC provider.**

## Rough Implementation Ideas

### 1. OIDC Discovery

Fetch `https://{issuer}/.well-known/openid-configuration` and extract:

- `authorization_endpoint`
- `token_endpoint`
- `scopes_supported` (intersect with what we need: `openid`, `email`, `profile`)
- `code_challenge_methods_supported` (confirm PKCE/S256)
- `token_endpoint_auth_methods_supported` (detect whether client secret is needed)
- `userinfo_endpoint` (for extracting email/name post-auth)

This is a single GET request that gives us everything we need to drive the existing `GenericOAuthProvider` flow.

### 2. User-Facing Flow

Two entry points:

**A. Issuer URL entry** - User provides their OIDC issuer URL (e.g., `https://auth.example.com/realms/corp`). We run discovery, populate everything, and the user just authenticates. Minimal friction for IT-managed deployments where users know their SSO URL.

**B. Email-based discovery** - User enters their email. We try:
1. Hardcoded registry (existing behavior for Gmail, Microsoft, etc.)
2. Mozilla autoconfig / SRV records (existing discovery)
3. **New**: WebFinger or `.well-known/openid-configuration` on the email domain
4. **New**: Prompt for manual OIDC issuer URL if all else fails

For on-prem deployments, IT can publish a Mozilla autoconfig XML that includes the OAuth2 endpoints, or set up WebFinger. But the manual issuer URL fallback means it works even with zero server-side discovery configuration.

### 3. Client Registration

This is the hardest UX problem. Unlike Google/Microsoft where we ship a client ID, custom OIDC providers require the client to be registered. Options:

**A. Pre-registered client ID** - The IT admin registers Ratatoskr as a client in their IdP and gives the user a client ID (and optionally secret). We provide documentation and a recommended configuration (redirect URI: `http://localhost:17248/callback`, grant type: authorization code, PKCE enabled). This is the most common pattern for desktop apps against corporate SSO.

**B. RFC 7591 Dynamic Client Registration** - If the OIDC provider supports it, we can register automatically. Check `registration_endpoint` in the discovery document. This is elegant but not universally supported.

**C. Public client with PKCE only** - Some providers allow public clients (no client secret) with PKCE. This is the OAuth2 for Native Apps recommendation (RFC 8252). We should try this first and only prompt for client ID/secret if needed.

Realistically, we should support all three in priority order: try dynamic registration, fall back to public client, fall back to manual client ID entry.

### 4. IMAP/SMTP Server Configuration

The OIDC provider handles authentication, but we still need IMAP/SMTP server addresses. These are separate from the IdP. Options:

- If discovered via autoconfig/SRV, we already have them.
- If the user entered an issuer URL directly, prompt for IMAP/SMTP server details (host, port, security).
- Consider a combined "corporate email setup" form: issuer URL + mail server + client ID.

### 5. Token-to-IMAP Mapping

The OAuth2 access token from the IdP needs to be accepted by Dovecot/Postfix. This is a server-side configuration concern, but we need to be aware of the token format:

- **XOAUTH2**: `user={email}\x01auth=Bearer {access_token}\x01\x01` - already implemented
- **OAUTHBEARER** (RFC 7628): More modern, slightly different format - worth adding as SASL mechanism detection from IMAP CAPABILITY response

We should check the IMAP server's CAPABILITY response for `AUTH=XOAUTH2` vs `AUTH=OAUTHBEARER` and use the appropriate mechanism.

### 6. Scope Negotiation

Different OIDC providers expose different scopes. We need `openid` and `email` at minimum. Some providers may require custom scopes for IMAP access (Microsoft uses `https://outlook.office.com/IMAP.AccessAsUser.All`).

For generic providers, start with `openid email profile` and allow the user (or IT admin) to specify additional scopes if their mail server requires a specific audience or scope claim.

### 7. Token Lifetime & Refresh

Most of this already works. One consideration for on-prem: refresh token rotation policies vary wildly. Some providers issue single-use refresh tokens. Our existing refresh logic handles this (we store the new refresh token from each exchange), but we should be careful about:

- Retry behavior on refresh failure (don't burn a single-use refresh token on a transient network error)
- Grace period before refresh (already 5 minutes - reasonable)
- Re-authentication flow when refresh tokens expire entirely

## Standards Reference

- **RFC 8252** - OAuth 2.0 for Native Apps (our overall model)
- **RFC 7636** - PKCE (already implemented)
- **RFC 7628** - SASL OAUTHBEARER (need to add alongside XOAUTH2)
- **RFC 7591** - Dynamic Client Registration (nice to have)
- **OpenID Connect Discovery 1.0** - `.well-known/openid-configuration`
- **RFC 7033** - WebFinger (for email-based discovery)
- **Mozilla Autoconfig** - ISP database / autoconfig XML (already integrated)

## Open Questions

- Should we support SASL OAUTHBEARER in addition to XOAUTH2, or is XOAUTH2 sufficient for the on-prem providers we're targeting?
- How much of the "corporate setup" flow should be configurable via a JSON/TOML file that IT departments can distribute? (e.g., a `ratatoskr-config.json` that pre-fills issuer, mail servers, client ID)
- Do we want to support OAuth2 for SMTP submission as well, or is IMAP-only sufficient for the first pass?
- Should we validate ID token signatures (requires fetching JWKS), or is the fact that we're exchanging an auth code over TLS sufficient trust?
