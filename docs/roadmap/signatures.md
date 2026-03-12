# Roaming Signatures

**Tier**: 2 — Keeps users from going back
**Status**: ⚠️ **Partial** — Local signature editor and DB storage (`signatures` table) are fully functional: create/edit/delete, per-account defaults, HTML body, sort order. **Missing**: server-side sync — no fetch from Exchange Graph roaming settings or Gmail `users.settings.sendAs` signatures. First-run experience doesn't auto-populate.

---

- **What**: Signatures stored server-side, synced across clients

## Cross-provider behavior

| Provider | Native support | API |
|---|---|---|
| Exchange (Graph) | Roaming signatures (relatively new, ~2021) | Graph beta endpoints / EWS roaming settings |
| Gmail API | Signature in settings | `users.settings.sendAs` — per-alias signatures |
| JMAP | Nothing standardized | N/A |
| IMAP | Nothing | N/A |

## Pain points

- First-run experience: user adds their Exchange account, expects their signature to appear in compose automatically. If we don't fetch it, they have to manually recreate it — immediate negative impression.
- HTML signatures: signatures are rich HTML (logos, formatted text, links). Need to render them in compose and handle the boundary between user-typed content and the signature block.
- Multiple signatures: Exchange supports multiple signatures (new email vs reply). Gmail supports per-alias signatures. Need a signature picker or smart default (use reply signature for replies, new-email signature for new compose).
- JMAP/IMAP accounts: purely local signatures. Need a signature editor that stores locally. Same UI, just no server sync.
- Signature images: signatures often contain inline images (company logos, headshots). These are the 14KB PNGs that compound at volume. When fetching a roaming signature, need to extract inline images and deduplicate them in the attachment store.
- Corporate-managed signatures: some orgs push signatures via Exchange transport rules (appended server-side on send). Client-side signature would double up. Need to detect this — if the server appends a signature, don't insert one client-side. Hard to detect reliably.

## Work

Fetch server-side signature on account setup for Exchange/Gmail, local signature editor for all accounts, handle HTML signatures in compose, smart default selection for reply vs new.
