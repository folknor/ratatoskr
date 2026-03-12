# Cloud Attachment Linking (OneDrive / Google Drive)

**Tier**: 1 — Blocks switching from Outlook
**Status**: ❌ **Not implemented**

---

- **What**: Attachments above a size threshold uploaded to cloud storage, shared as links instead of inline

## Cross-provider behavior

| Provider | Cloud storage | Auto-linking |
|---|---|---|
| Exchange (Graph) | OneDrive via `/me/drive` | Outlook auto-converts large attachments to OneDrive links |
| Gmail API | Google Drive | Gmail prompts for Drive link above 25MB |
| JMAP | None built-in | N/A |
| IMAP | None built-in | N/A |

## Pain points

- Incoming link detection: users receive emails with OneDrive/Google Drive/SharePoint links that should render as "attachments" in the UI, not as raw URLs in the body. Need URL pattern detection for major cloud providers and rendering them as downloadable attachment chips.
- Permission management: uploading to OneDrive and sharing a link requires setting permissions (org-wide? specific recipients? anyone with link?). Defaulting wrong is either a security issue (too open) or a usability issue (recipient can't access).
- Offline compose: user composes offline with a large attachment. Can't upload to OneDrive yet. Need to queue the upload and convert to link on send when connectivity returns.
- JMAP/IMAP accounts: no cloud storage integration. Options are: (a) just send the large file if the server allows it, (b) warn the user about size limits, (c) offer a local integration with a third-party storage provider (complex, probably out of scope initially).
- Mixed accounts in compose: user has an Exchange account and a Stalwart account. Compose defaults to Exchange sender — cloud linking works. They switch sender to Stalwart mid-compose — cloud linking no longer available. UI needs to handle this gracefully.

## Work

OneDrive upload for Exchange accounts, Google Drive for Gmail accounts, incoming link detection across all providers, graceful degradation for JMAP/IMAP.
