# BIMI (Brand Indicators for Message Identification)

**Tier**: 3 — Differentiators and polish
**Status**: ❌ **Not implemented**

---

- **What**: Verified sender brand logos displayed next to messages from authenticated domains
- **Scope**: Client-side only — DNS lookup + header check, works identically across all providers

## Pain points

- Performance: every unique sender domain requires a DNS TXT lookup + potential SVG fetch. At hundreds of emails/day with diverse senders, this is a lot of lookups. Need aggressive caching per domain (logos don't change often — cache for days/weeks).
- Validation: BIMI requires DMARC pass. Must check `Authentication-Results` header for DMARC status. If the header is missing or DMARC failed, don't display the logo (it's unverified).
- SVG rendering: BIMI logos are SVG Tiny PS (a restricted SVG profile). Need an SVG renderer that handles this subset. Full SVG renderers may work, but the spec is specific about what's allowed.
- VMC (Verified Mark Certificate): full BIMI validation requires checking a VMC certificate (X.509 with the logo embedded). This is the "verified" part. Without VMC checking, you can still display the logo but can't claim it's verified. VMC checking is complex (certificate chain validation, embedded logo comparison). Start without VMC, add later.
- Fallback: for domains without BIMI, fall back to colored initials or gravatar. BIMI should feel like an enhancement over the default avatar, not a required element.

## Work

DNS BIMI record lookup, SVG logo fetch + cache per domain, check DMARC pass in `Authentication-Results`, display in sender avatar slot. Skip VMC validation initially.
