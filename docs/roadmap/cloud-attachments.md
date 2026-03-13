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

## Research

**Date**: March 2026

---

### Table of Contents

- [OneDrive API via Microsoft Graph](#onedrive-api-via-microsoft-graph)
- [Google Drive API v3](#google-drive-api-v3)
- [SharePoint Document Libraries](#sharepoint-document-libraries)
- [Exchange Reference Attachments](#exchange-reference-attachments)
- [Gmail Drive Attachment Behavior](#gmail-drive-attachment-behavior)
- [JMAP Blob Upload](#jmap-blob-upload)
- [Incoming Link Detection](#incoming-link-detection)
- [Rust Crates for Cloud Storage](#rust-crates-for-cloud-storage)
- [What Other Clients Do](#what-other-clients-do)
- [Data Model](#data-model)
- [Offline Queue Architecture](#offline-queue-architecture)
- [Recommendations](#recommendations)

---

### OneDrive API via Microsoft Graph

All OneDrive operations go through the same Microsoft Graph endpoints we already use for Exchange mail. Two upload paths exist depending on file size.

#### Small file upload (< 4 MB)

Simple PUT to `/me/drive/items/{parent-id}:/{filename}:/content`. Single request, single response with the created `driveItem`. No session management needed.

```
PUT /me/drive/items/{parent-id}:/{filename}:/content
Content-Type: application/octet-stream

<file bytes>
```

#### Resumable upload (>= 4 MB, max 250 GB)

Two-phase process via upload sessions:

1. **Create session**: `POST /me/drive/items/{parent-id}:/{filename}:/createUploadSession` with optional conflict behavior (`rename`, `replace`, `fail`). Returns an `uploadUrl` and `expirationDateTime`.

2. **Upload fragments**: Sequential `PUT` requests to the `uploadUrl` with `Content-Range` headers. Fragment size **must be a multiple of 320 KiB** (327,680 bytes). Recommended fragment size: 5-10 MiB. Maximum per-request: 60 MiB. The `uploadUrl` is pre-authenticated — do **not** include the `Authorization` header on PUT requests.

3. **Completion**: Automatic when the last byte range is received (`deferCommit: false`, the default). Returns the completed `driveItem`. Sessions expire if no fragments are received within the `expirationDateTime` window; each successful fragment upload extends the expiration.

4. **Resume**: `GET` to the `uploadUrl` returns `nextExpectedRanges` listing missing byte ranges. The client can resume from there after a connection drop.

5. **Cancel**: `DELETE` to the `uploadUrl` discards all uploaded fragments.

**Required scopes**: `Files.ReadWrite` (delegated, least privilege) or `Files.ReadWrite.All`, `Sites.ReadWrite.All`.

#### Creating sharing links

`POST /me/drive/items/{item-id}/createLink` with a JSON body specifying link type and scope:

```json
{
  "type": "view",       // "view" | "edit" | "embed" (embed = OneDrive Personal only)
  "scope": "organization", // "anonymous" | "organization" | "users"
  "password": "...",    // OneDrive Personal only
  "expirationDateTime": "2026-04-01T00:00:00Z"
}
```

**Scope types**:

| Scope | Description | Availability |
|---|---|---|
| `anonymous` | Anyone with the link, no sign-in required | Personal + Business (admin can disable) |
| `organization` | Anyone in the tenant | OneDrive for Business / SharePoint only |
| `users` | Specific people only | OneDrive for Business / SharePoint only |

If `scope` is omitted, the organization default is used. For enterprise accounts this is typically `organization` — a safe default for Ratatoskr since most enterprise admins disable anonymous links anyway.

**Response**: A `Permission` resource with `link.webUrl` containing the shareable URL (e.g., `https://1drv.ms/...` or `https://contoso-my.sharepoint.com/...`).

Links do not expire unless an organization policy enforces it. The `expirationDateTime` property is optional.

**Required scopes**: Same as upload — `Files.ReadWrite`.

#### Size thresholds

Outlook auto-converts attachments above **a tenant-configurable threshold** (default varies, commonly 20-25 MB for Outlook desktop, but organization policy can change it). We should default to a 10 MB threshold for cloud linking, configurable per account in settings.

---

### Google Drive API v3

#### Upload methods

Three upload types, all via `POST https://www.googleapis.com/upload/drive/v3/files`:

| Method | `uploadType` param | Max size | Use case |
|---|---|---|---|
| Simple | `media` | 5 MB | Small files, no metadata |
| Multipart | `multipart` | 5 MB | Small files + metadata in one request |
| Resumable | `resumable` | 5 TB | Large files, reliable transfer |

**Resumable upload flow**:

1. **Initiate**: `POST` with `uploadType=resumable`, file metadata in body. Returns a `Location` header with a resumable URI.
2. **Upload chunks**: `PUT` to the resumable URI with `Content-Range` headers. No strict chunk alignment requirement like OneDrive, but chunking is recommended for files > 5 MB.
3. **Resume**: `PUT` with `Content-Range: bytes */{total}` to query progress, then resume from the next byte.

**OAuth scopes**:

| Scope | Access | Sensitivity |
|---|---|---|
| `drive.file` | Files created/opened by the app only | Non-sensitive (preferred) |
| `drive` | All Drive files | Sensitive, requires verification |

`drive.file` is strongly preferred. It gives Ratatoskr access only to files it creates (the uploaded attachments) without touching the user's entire Drive. Google's OAuth consent screen is simpler with non-sensitive scopes.

#### Creating sharing permissions

`POST https://www.googleapis.com/drive/v3/files/{fileId}/permissions` with:

```json
{
  "role": "reader",           // "reader" | "writer" | "commenter" | "owner"
  "type": "anyone",           // "anyone" | "domain" | "user" | "group"
  "emailAddress": "...",      // required when type = "user" or "group"
  "domain": "...",            // required when type = "domain"
  "allowFileDiscovery": false  // whether the file appears in Drive search
}
```

For email cloud attachments, the typical permission is `{ "role": "reader", "type": "anyone" }` — anyone with the link can view. For enterprise Google Workspace, `{ "role": "reader", "type": "domain", "domain": "company.com" }` restricts to the organization.

**Required scope**: `drive.file` or `drive` (same as for upload).

#### Gmail's native behavior

When a Gmail user attaches a file > 25 MB, Gmail web UI prompts to upload to Google Drive and insert a link. This is a **client-side behavior** — the Gmail API itself does not automatically convert large attachments to Drive links. To replicate this in Ratatoskr, we must handle it ourselves: detect the attachment size, upload to Drive via the Drive API, create a sharing permission, and insert the link into the message body.

The Gmail API upload limit for inline attachments is 25 MB for the simple upload method and 35 MB total message size via the resumable upload method.

---

### SharePoint Document Libraries

SharePoint document libraries use the **same Graph API `driveItem` surface** as OneDrive. The difference is the path prefix:

| Storage | API path |
|---|---|
| User's OneDrive | `/me/drive/items/...` |
| SharePoint site library | `/sites/{site-id}/drive/items/...` |
| Group drive | `/groups/{group-id}/drive/items/...` |

Upload, sharing links, and permissions work identically. A `driveItem` in SharePoint is the same resource type as in OneDrive.

**For incoming mail**: SharePoint links in email bodies use domains like `{tenant}.sharepoint.com` (e.g., `contoso.sharepoint.com/sites/...`). These are the same patterns we need to detect for OneDrive for Business links — OneDrive for Business URLs use `{tenant}-my.sharepoint.com/personal/...`.

**For outgoing mail**: We only need to support uploading to the user's OneDrive (`/me/drive`). Supporting upload to arbitrary SharePoint document libraries would require site enumeration and selection UI — out of scope for initial implementation, but the API surface is identical if we add it later.

---

### Exchange Reference Attachments

When Outlook creates a cloud-linked attachment, it creates a `referenceAttachment` resource on the message via Graph API. This is a first-class attachment type alongside `fileAttachment` and `itemAttachment`.

**Critical limitation**: The `referenceAttachment` resource type exists in both v1.0 and beta, but the **writable properties** (`sourceUrl`, `providerType`, `permission`) are only available in the **beta** endpoint. The v1.0 endpoint only exposes read-only properties (`id`, `name`, `size`, `contentType`, `isInline`, `lastModifiedDateTime`).

**Beta properties**:

| Property | Type | Description |
|---|---|---|
| `sourceUrl` | String | URL to the cloud file (required) |
| `providerType` | Enum | `oneDriveBusiness`, `oneDriveConsumer`, `dropbox`, `other` |
| `permission` | Enum | `view`, `edit`, `anonymousView`, `anonymousEdit`, `organizationView`, `organizationEdit`, `other` |
| `isFolder` | Boolean | Whether the link points to a folder |
| `thumbnailUrl` | String | Preview image URL (for image files) |
| `previewUrl` | String | Preview image URL (for image files) |

**To create a reference attachment via beta**:

```
POST /beta/me/messages/{message-id}/attachments
Content-Type: application/json

{
  "@odata.type": "#microsoft.graph.referenceAttachment",
  "name": "report.docx",
  "sourceUrl": "https://contoso-my.sharepoint.com/personal/.../report.docx",
  "providerType": "oneDriveBusiness",
  "permission": "organizationView",
  "isFolder": false
}
```

**Practical implications**:

1. **Reading reference attachments**: We can detect them via v1.0 GET — they have `@odata.type: "#microsoft.graph.referenceAttachment"`. But v1.0 lacks `sourceUrl`, so we cannot retrieve the cloud file URL without using the beta endpoint.

2. **Creating reference attachments**: Requires beta endpoint. This is how Outlook natively creates cloud attachments. We should use this when sending from Exchange accounts rather than inserting raw URLs into the message body. The recipient's Outlook will render them as proper attachment chips.

3. **Beta API risk**: Beta APIs can change without notice. However, `referenceAttachment` has been in beta since ~2016 without breaking changes. Microsoft has not promoted it to v1.0 in 10 years, suggesting it will stay in beta indefinitely but remain stable.

**Recommendation**: Use the beta endpoint for both reading and creating reference attachments on Exchange accounts. Fall back to URL detection for other providers or if the beta call fails.

---

### Gmail Drive Attachment Behavior

Gmail does not have an equivalent of `referenceAttachment`. When a user inserts a Google Drive link in Gmail web, the result is simply **an HTML link in the message body** — there is no special MIME part or metadata structure. Gmail's UI parses the message body to detect Drive links and renders them as attachment chips on the receiving end.

This means:

1. **Sending from Gmail accounts**: We upload to Drive, create a sharing permission, and insert an HTML link (`<a href="https://drive.google.com/file/d/...">filename.ext</a>`) into the message body. No special API for "Drive attachments" exists.

2. **Receiving Gmail messages with Drive links**: The links are just regular `<a>` tags in the HTML body. We detect them via URL pattern matching (see [Incoming Link Detection](#incoming-link-detection)).

3. **No metadata enrichment on send**: Unlike Exchange's `referenceAttachment` where the server stores file metadata (provider type, permissions, thumbnail), Gmail just has a URL. To show file size, icon, etc., we must fetch metadata from the Drive API when rendering (or cache it at send time).

---

### JMAP Blob Upload

JMAP (RFC 8620) has a built-in blob upload mechanism. The JMAP session object advertises `maxSizeUpload` — the maximum blob size the server accepts, in octets.

**Stalwart configuration** (the primary JMAP server we target):

| Setting | Key | Example value |
|---|---|---|
| Max upload size | `jmap.protocol.upload.max-size` | 50,000,000 (50 MB) |
| Max concurrent uploads | `jmap.protocol.upload.max-concurrent` | 4 |
| Max attachment size | `jmap.email.max-attachment-size` | 50,000,000 (50 MB) |
| Max message size | `jmap.email.max-size` | 75,000,000 (75 MB) |

The JMAP `Session` resource reports `maxSizeUpload` in `urn:ietf:params:jmap:core` capabilities. `jmap-client` exposes this via the session object after connection.

**RFC 9404** (JMAP Blob Management Extension) adds more advanced blob operations (lookup by hash, upload from URL), but this is not widely implemented and not needed for basic attachment handling.

**Practical implications for JMAP/IMAP accounts**: These accounts have no cloud storage integration. The attachment goes inline in the message, subject to server size limits. We should:

1. Check `maxSizeUpload` from the JMAP session (or fall back to a conservative 25 MB for IMAP).
2. Warn the user if the attachment exceeds the limit.
3. Do **not** offer "upload to cloud" for JMAP/IMAP accounts in v1 — there is no user-associated cloud storage to upload to. A future extension could integrate with user-configured Nextcloud, but that is out of scope.

---

### Incoming Link Detection

Users receive emails containing cloud storage links from other people's clients. These should render as downloadable attachment chips in the UI, not as raw URLs buried in HTML.

#### URL patterns to detect

| Provider | Domains | Path patterns |
|---|---|---|
| **OneDrive Personal** | `1drv.ms`, `onedrive.live.com` | Short links (`1drv.ms/...`), `/redir?resid=...` |
| **OneDrive Business** | `{tenant}-my.sharepoint.com` | `/personal/{user}/_layouts/15/...`, `/personal/{user}/Documents/...` |
| **SharePoint** | `{tenant}.sharepoint.com` | `/sites/{site}/...`, `/:w:/s/{site}/...` |
| **Google Drive** | `drive.google.com`, `docs.google.com` | `/file/d/{id}/...`, `/document/d/{id}/...`, `/spreadsheets/d/{id}/...`, `/presentation/d/{id}/...`, `/open?id={id}` |
| **Dropbox** | `dropbox.com`, `dl.dropboxusercontent.com` | `/s/{key}/...`, `/scl/fi/{key}/...` |
| **Box** | `app.box.com` | `/s/{token}`, `/file/{id}` |

#### Implementation approach

A `regex::RegexSet` compiled once at startup, matching against `href` attributes extracted from the HTML body by the existing `mail-parser` / HTML sanitizer pipeline. We are not scanning raw text — we scan parsed `<a>` tags only, which avoids false positives on partial URL matches in body text.

```rust
// Conceptual — compile once, match many
let cloud_link_patterns = RegexSet::new(&[
    r"https?://1drv\.ms/",
    r"https?://onedrive\.live\.com/",
    r"https?://[a-z0-9-]+-my\.sharepoint\.com/personal/",
    r"https?://[a-z0-9-]+\.sharepoint\.com/",
    r"https?://drive\.google\.com/(file|open)",
    r"https?://docs\.google\.com/(document|spreadsheets|presentation|forms)/d/",
    r"https?://(www\.)?dropbox\.com/s(cl)?/",
    r"https?://dl\.dropboxusercontent\.com/",
    r"https?://app\.box\.com/(s|file)/",
])?;
```

#### Metadata enrichment

When a cloud link is detected, we optionally fetch metadata to display a rich attachment chip:

| Provider | Metadata source | Auth required? |
|---|---|---|
| OneDrive / SharePoint | Graph API `GET /shares/{encoded-url}/driveItem` | Yes (user must have a Microsoft account in Ratatoskr) |
| Google Drive | Drive API `GET /files/{id}?fields=name,size,mimeType,iconLink` | Yes (user must have a Google account in Ratatoskr) |
| Dropbox, Box | No public metadata API for shared links without auth | No practical way to enrich |

**Fallback**: If no authenticated account is available or the API call fails, display the link text (or filename extracted from URL path) with a generic cloud icon. The user can still click to open in browser.

**Important caveat about Outlook behavior**: As of 2024, Outlook on the web and new Outlook for Windows insert cloud attachments as **inline shared links in the HTML body** rather than as `referenceAttachment` objects. This means even for Exchange-sourced mail, link detection in the HTML body is necessary — we cannot rely solely on the Graph attachment API to find cloud attachments.

---

### Rust Crates for Cloud Storage

#### Microsoft Graph / OneDrive

| Crate | Version | Downloads | Maintained | Assessment |
|---|---|---|---|---|
| [`graph-rs-sdk`](https://crates.io/crates/graph-rs-sdk) | 3.0.1 | 77K total, 8.5K recent | Yes (145 stars) | Full Graph SDK. Covers drives, upload sessions, sharing. Heavy dependency (pulls in wry/tao for webview auth). v3.0.1 failed to build on docs.rs. |
| [`onedrive-api`](https://crates.io/crates/onedrive-api) | 0.11.0 | 35K total, 942 recent | Low activity (42 stars) | Focused OneDrive binding. Upload sessions, permissions, change tracking. Lighter than graph-rs-sdk. No sharing link creation documented. |

**Verdict**: Neither crate is a clear win. `graph-rs-sdk` is comprehensive but heavy, has build issues, and pulls in webview dependencies we do not want (Ratatoskr uses iced, not webview). `onedrive-api` is lighter but does not clearly cover the `createLink` endpoint we need for sharing.

**Recommendation**: Use **raw reqwest** against the Graph API, same as our existing Exchange provider. We already have Graph API authentication, token refresh, and request infrastructure in `ratatoskr-core`. Adding OneDrive upload + sharing is a handful of endpoints — the overhead of an SDK crate is not justified. The upload session flow is ~100 lines of Rust (create session, loop uploading chunks, handle resume). The sharing link call is a single POST.

#### Google Drive

| Crate | Version | Downloads | Maintained | Assessment |
|---|---|---|---|---|
| [`google-drive3`](https://crates.io/crates/google-drive3) | 7.0.0+20251218 | 1.6M total, 352K recent | Yes (auto-generated) | Official Google-generated binding. Covers all Drive v3 endpoints. Part of the `google-apis-rs` project by Sebastian Thiel (Byron). Auto-generated from the API discovery document. |
| [`google-drive`](https://crates.io/crates/google-drive) | 0.10.0 | 224K total, 770 recent | Low activity | Opinionated wrapper. Lower-level than google-drive3. |

**`google-drive3` assessment**: This is the only crate worth considering. 1.6M downloads proves real-world usage. Auto-generated means API coverage is complete and stays current. The downside is typical of Google's generated SDKs — verbose API, large compile times, and the generated code can be awkward to use. It depends on `hyper` and `yup-oauth2` which may duplicate our existing OAuth infrastructure.

**Recommendation**: Evaluate `google-drive3` for the Drive upload + permission flow. If the dependency tree conflicts with our existing reqwest-based Gmail provider, fall back to raw reqwest against the Drive REST API (same pattern as OneDrive). The Drive API surface we need is small: `files.create` (upload), `permissions.create` (sharing), `files.get` (metadata for incoming link enrichment).

---

### What Other Clients Do

#### Thunderbird — FileLink

Thunderbird's cloud attachment system is called **FileLink**. It is a plugin architecture:

- Core Thunderbird ships with no cloud providers built-in.
- Providers are added via MailExtension add-ons. A third-party "Thunderbird-CloudLink" add-on supports OneDrive and Google Drive, with optional password protection and expiration dates.
- When composing, if an attachment exceeds a configurable threshold (default 5 MB), Thunderbird prompts to convert it to a FileLink. The add-on uploads the file and inserts a standardized HTML template into the message body with download links.
- FileLink produces a **text/html body part** with a specific template. Other Thunderbird users see this rendered as a download card. Non-Thunderbird recipients see the HTML with links.

**Key takeaway**: Thunderbird does not use Exchange `referenceAttachment` or any provider-specific attachment metadata. It is purely HTML body injection, which means maximum compatibility but no rich integration with Outlook's attachment UI.

#### eM Client

eM Client has the most mature cloud attachment implementation among third-party desktop clients:

- Supports OneDrive, Google Drive, Dropbox, OwnCloud, and Nextcloud.
- Creates a dedicated folder (`eM Client Attachments`) in the user's cloud storage.
- Right-click any local attachment to "Upload to cloud storage" and convert to a link.
- Supports password protection and expiration dates on download links.
- Configurable via Settings > Mail > Attachments — user adds cloud providers with OAuth consent.
- Size threshold is configurable per provider.

**Key takeaway**: eM Client's model of a dedicated attachments folder in cloud storage is a good UX pattern — it keeps uploaded attachments organized and easy to find/clean up. We should adopt this: create a `Ratatoskr Attachments` folder in OneDrive / Google Drive on first use.

#### Mailspring

Mailspring does not have built-in cloud attachment support. It is a simpler client focused on compose enhancements (send later, templates, read receipts) rather than enterprise features like cloud storage integration.

---

### Data Model

Cloud attachment metadata needs to be stored locally for both outgoing (queued uploads) and incoming (detected links) messages.

#### Proposed schema

```sql
CREATE TABLE cloud_attachments (
    id TEXT PRIMARY KEY,           -- UUID
    message_id TEXT NOT NULL,      -- FK to messages table
    account_id TEXT NOT NULL,      -- FK to accounts table
    direction TEXT NOT NULL,       -- 'outgoing' | 'incoming'

    -- Cloud file metadata
    provider TEXT NOT NULL,        -- 'onedrive' | 'google_drive' | 'dropbox' | 'box' | 'sharepoint' | 'unknown'
    cloud_url TEXT NOT NULL,       -- The sharing URL
    original_filename TEXT,        -- Original filename
    file_size INTEGER,            -- File size in bytes (may be null for incoming if unenriched)
    mime_type TEXT,               -- MIME type
    icon_url TEXT,                -- Provider icon or thumbnail URL

    -- Permission metadata (outgoing only)
    permission_scope TEXT,        -- 'anonymous' | 'organization' | 'users' | null
    permission_role TEXT,         -- 'view' | 'edit' | null
    expiration TEXT,              -- ISO 8601 timestamp or null

    -- Upload tracking (outgoing only)
    upload_status TEXT,           -- 'pending' | 'uploading' | 'uploaded' | 'failed'
    drive_item_id TEXT,           -- Provider-specific file ID after upload
    upload_session_url TEXT,      -- Resumable upload URL (transient)
    bytes_uploaded INTEGER,       -- Progress tracking for resumable uploads

    -- Timestamps
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_cloud_attachments_message ON cloud_attachments(message_id);
CREATE INDEX idx_cloud_attachments_status ON cloud_attachments(upload_status)
    WHERE upload_status IN ('pending', 'uploading');
```

This table lives in the main `ratatoskr.db` (not in `bodies.db` — cloud attachment metadata is small and queried alongside message metadata).

The `upload_session_url` is transient — it is only valid during an active upload session and should be cleared on app restart (sessions expire server-side anyway). On restart, any `uploading` status entries should be reset to `pending` for retry.

---

### Offline Queue Architecture

When the user composes offline with a large attachment destined for cloud upload:

#### State machine

```
  [local_file_attached]
          |
          v
  [pending]  -- file selected, cloud upload needed, no connectivity
          |  (connectivity restored or was never lost)
          v
  [uploading]  -- upload session created, chunks being sent
          |
          +---> [failed]  -- network error, quota exceeded, etc.
          |        |
          |        v  (auto-retry with backoff, or manual retry)
          |     [pending]
          |
          v
  [uploaded]  -- file in cloud, sharing link obtained
          |
          v
  [linked]  -- sharing link inserted into message, ready to send
```

#### Key design decisions

1. **Attachment stored locally until upload completes**: The local file (or a copy) must be retained until the cloud upload succeeds. If the user deletes the local file before upload, the attachment is lost. Store a copy in the app's data directory.

2. **Upload happens at send time, not compose time**: Uploading eagerly during compose wastes bandwidth if the user discards the draft. Upload when the user hits Send. If offline, queue the entire send operation (including the upload step).

3. **Compose model holds both representations**: The compose state holds the local file path and a `CloudAttachmentState` enum. The send pipeline checks: if `upload_status == uploaded`, insert the sharing link and send. If `pending`, run the upload first. If the upload fails, abort the send and surface the error.

4. **Sender switching mid-compose**: If the user switches sender from an Exchange account (cloud-capable) to a JMAP account (not cloud-capable):
   - If the attachment is small enough for inline, silently convert back to inline attachment.
   - If the attachment exceeds the JMAP server's `maxSizeUpload`, warn the user that the attachment is too large for the selected sender and cannot be cloud-linked.

5. **Resumable upload recovery**: On app restart, scan for `cloud_attachments` rows with `upload_status = 'uploading'`. For each, check if the upload session is still valid (`GET` to the session URL). If valid, resume. If expired, restart from `pending`.

6. **Deduplication**: If the same file is attached to multiple drafts, upload once and share the same cloud file. Key on file hash + account to detect duplicates.

---

### Recommendations

#### Phase 1: OneDrive for Exchange accounts (highest value)

This is the Tier 1 blocker. Enterprise Outlook users constantly send and receive OneDrive/SharePoint links.

1. **Outgoing**: Upload via Graph API resumable upload to `/me/drive/items/root:/Ratatoskr Attachments/{filename}:/createUploadSession`. Create an `organization`-scoped `view` link via `createLink`. Create a `referenceAttachment` on the message via the beta endpoint.

2. **Incoming**: Detect OneDrive/SharePoint URLs in HTML body via regex. Fetch `driveItem` metadata via the Graph sharing API for enriched display. Fall back to link text + generic icon.

3. **Threshold**: Default 10 MB, configurable per account. Prompt user on attach: "Upload to OneDrive and share as link?" with a "Always do this for files over X MB" checkbox.

Raw reqwest against Graph API — no SDK crate dependency.

#### Phase 2: Google Drive for Gmail accounts

Same flow as OneDrive but against the Drive API v3:

1. **Outgoing**: Upload via resumable upload to Drive. Create `{ "role": "reader", "type": "anyone" }` permission. Insert `<a>` link into message HTML body (no `referenceAttachment` equivalent).

2. **Incoming**: Detect `drive.google.com` / `docs.google.com` URLs. Fetch file metadata via Drive API for enrichment.

3. **Scope**: Use `drive.file` — requires adding this scope to our existing Gmail OAuth flow. It is a non-sensitive scope, so no additional Google verification needed.

Evaluate `google-drive3` crate vs raw reqwest based on dependency tree impact.

#### Phase 3: Incoming link detection for all providers

Compile `RegexSet` for all major cloud providers (OneDrive, Google Drive, Dropbox, Box, SharePoint). Run on message render for all account types. Display detected links as attachment chips with provider icon + filename. Click opens in browser. No metadata enrichment for Dropbox/Box (no practical way without auth).

#### Out of scope (v1)

- Cloud storage for JMAP/IMAP accounts (no user-associated storage).
- SharePoint document library browsing/upload (only personal OneDrive).
- Dropbox/Box upload integration (low enterprise demand relative to OneDrive/Drive).
- Password-protected or expiring links (complexity, low initial value).
- File preview rendering within Ratatoskr (open in browser is sufficient).
