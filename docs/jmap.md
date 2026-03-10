# JMAP Provider Implementation Reference

This document captures the full upstream JMAP implementation from [velo commit 5fae5dd](https://github.com/avihaymenahem/velo/commit/5fae5dd8f2888572830a6ca9f2ae8f5b19ea1e9e) for future porting. JMAP (RFC 8620/8621) is a modern JSON-over-HTTP email protocol supported by Fastmail, Stalwart, Cyrus IMAP v3.8+, and other self-hosted servers.

## Architecture Overview

JMAP is implemented as a third `EmailProvider` alongside Gmail API and IMAP/SMTP. The key difference from IMAP is that JMAP is stateless HTTP — no persistent connections, no UIDs, no folder SELECT. Instead it uses opaque state strings for delta sync and mailbox IDs for folder membership.

### File Structure (11 source + 6 test files)

| File | Purpose |
|------|---------|
| `src/services/jmap/types.ts` | Full JMAP type definitions (Session, Email, Mailbox, BodyPart, Request/Response) |
| `src/services/jmap/client.ts` | `JmapClient` class — session discovery, auth, batched API calls, blob ops |
| `src/services/jmap/clientFactory.ts` | Creates `JmapClient` from `DbAccount` record |
| `src/services/jmap/autoDiscovery.ts` | `.well-known/jmap` + known provider list (Fastmail) |
| `src/services/jmap/mailboxMapper.ts` | JMAP mailbox roles ↔ Gmail-style labels, bidirectional |
| `src/services/jmap/jmapSync.ts` | Initial sync (batched) + delta sync (Email/changes) |
| `src/services/email/jmapProvider.ts` | Full `EmailProvider` implementation (all 17 methods) |
| `src/components/accounts/AddJmapAccount.tsx` | 3-step account setup UI |
| `src/services/db/jmapSyncState.ts` | Per-type sync state persistence |

### DB Migration (migration 18)

- `accounts.jmap_url` column (nullable) — stores the JMAP session URL
- `jmap_sync_state` table — tracks sync state per account per JMAP type (Email, Mailbox)

### Integration Points

- `providerFactory.ts` — routes `account.provider === "jmap"` to `JmapProvider`
- `syncManager.ts` — adds `syncJmapAccount()` with initial/delta routing
- `accounts.ts` — `DbAccount` gets `jmap_url` field
- `entities.mock.ts` — test mocks get `jmap_url: null`

## Key Design Decisions

1. **Auth**: Basic (base64 user:pass) or Bearer (OAuth2 token). Determined from `account.auth_method`.
2. **Session caching**: `JmapClient` caches the session resource and invalidates when `sessionState` changes in API responses.
3. **Mailbox → label mapping**: Same pattern as IMAP `folderMapper.ts`. Role-based mailboxes map to system labels (INBOX, SENT, etc.), user mailboxes get `jmap-{id}` prefix. Keywords ($seen, $flagged, $draft) map to pseudo-labels (UNREAD, STARRED, DRAFT).
4. **Delta sync**: Uses `Email/changes` and `Mailbox/changes` with `cannotCalculateChanges` error handling (falls back to full refresh). State strings stored per-type in `jmap_sync_state`.
5. **Send**: Uses `Email/import` (upload RFC 822 blob) + `EmailSubmission/set` in a single batched API call.
6. **Drafts**: `Email/import` with `$draft` + `$seen` keywords into drafts mailbox. Update = delete old + create new (JMAP has no draft mutation).
7. **Threading**: JMAP provides `threadId` natively — no JWZ threading needed (unlike IMAP).
8. **Bodies**: Fetched via `bodyValues` with `fetchTextBodyValues` + `fetchHTMLBodyValues` flags. No separate body store needed during sync since bodies come with the email object.

## Porting Notes for Ratatoskr

When porting to our codebase, consider:

- Our DB filename is `ratatoskr.db`, event names use `ratatoskr-*` prefix
- Our Rust sync engine handles IMAP — JMAP could stay in TS (it's HTTP-based, like Gmail) or get a Rust implementation
- The `EmailProvider` interface may have diverged — audit all 17 methods
- Need to add `jmap_url` to our `DbAccount` interface and Rust `db/types.rs`
- Need migration for `jmap_sync_state` table and `accounts.jmap_url` column
- The upstream uses `velo` sync interval of 15s — we use 60s
- Our body store (zstd-compressed `bodies.db`) would need integration — the upstream stores bodies directly in the messages table
- Filter engine, smart labels, notifications, and AI categorization post-sync hooks need to be wired in (same as our `syncImapAccountRust` pattern)
- CSP in `tauri.conf.json` needs to allow arbitrary JMAP server domains (or use Tauri HTTP plugin which bypasses CSP)

## Upstream Diff

The complete diff from commit `5fae5dd8f2888572830a6ca9f2ae8f5b19ea1e9e` is included below for reference. This adds JMAP as a third email provider with full sync, all email actions, account setup UI, and 152 tests across 6 test files.

Also relevant: commit `4e4f8f18` (IMAP UTF-7 folder names + sparse UID fix) which we already have — included at the end for completeness.

---

### JMAP Commit (5fae5dd)

```diff
diff --git a/CLAUDE.md b/CLAUDE.md
index f900470..adbba81 100755
--- a/CLAUDE.md
+++ b/CLAUDE.md
@@ -41,9 +41,10 @@ Tauri v2 desktop app: Rust backend + React 19 frontend communicating via Tauri I

 2. **Service layer** (`src/services/`): All business logic. Plain async functions (not classes, except `GmailClient`).
    - `db/` — SQLite queries via `getDb()` singleton from `connection.ts`. Version-tracked migrations in `migrations.ts`. FTS5 full-text search on messages (trigram tokenizer). 29 service files covering accounts, messages, threads, labels, contacts, filters, templates, signatures, attachments, scheduled emails, image allowlist, search, settings, AI cache, bundle rules, calendar events, follow-up reminders, notification VIPs, thread categories, send-as aliases, smart folders, quick steps, link scan results, phishing allowlist, and folder sync state.
-   - `email/` — `EmailProvider` abstraction unifying Gmail API and IMAP/SMTP behind a single interface. `providerFactory.ts` returns appropriate provider based on `account.provider` field ("gmail_api" or "imap"). `gmailProvider.ts` wraps existing GmailClient. `imapSmtpProvider.ts` delegates to Rust IMAP/SMTP Tauri commands.
+   - `email/` — `EmailProvider` abstraction unifying Gmail API, IMAP/SMTP, and JMAP behind a single interface. `providerFactory.ts` returns appropriate provider based on `account.provider` field ("gmail_api", "imap", or "jmap"). `gmailProvider.ts` wraps existing GmailClient. `imapSmtpProvider.ts` delegates to Rust IMAP/SMTP Tauri commands. `jmapProvider.ts` implements JMAP (RFC 8620/8621) via JSON-over-HTTP.
    - `gmail/` — `GmailClient` class auto-refreshes tokens 5min before expiry, retries on 401. `tokenManager.ts` caches clients per account in a Map. `syncManager.ts` orchestrates sync (60s interval) for both Gmail and IMAP accounts via the EmailProvider abstraction. `sync.ts` does initial sync (365 days, configurable via `sync_period_days` setting) and delta sync via Gmail History API; falls back to full sync if history expired (~30 days). `authParser.ts` parses SPF/DKIM/DMARC from `Authentication-Results` headers. `sendAs.ts` fetches send-as aliases from Gmail API.
+   - `jmap/` — JMAP (RFC 8620/8621) client and sync services. `client.ts` manages session discovery, authenticated API calls (Basic/Bearer), session state tracking, blob upload/download. `jmapSync.ts` handles initial sync (batched Email/query + Email/get) and delta sync (Email/changes + Mailbox/changes). `mailboxMapper.ts` maps JMAP mailbox roles to Gmail-style labels. `autoDiscovery.ts` provides `.well-known/jmap` discovery and known provider configs (Fastmail). `clientFactory.ts` creates authenticated clients from DB account records.
    - `imap/` — IMAP-specific services. `tauriCommands.ts` wraps Rust IMAP Tauri commands. `imapSync.ts` orchestrates IMAP initial sync (batch fetch, 50 messages/batch) and delta sync via UIDVALIDITY/last_uid tracking. `folderMapper.ts` maps IMAP folders (special-use flags + well-known names) to Gmail-style labels. `autoDiscovery.ts` provides pre-configured server settings for 7 major providers (Outlook, Yahoo, iCloud, AOL, Zoho, FastMail, GMX). `imapConfigBuilder.ts` builds IMAP/SMTP configs from account records. `messageHelper.ts` handles IMAP message utilities.
diff --git a/src/components/accounts/AddAccount.tsx b/src/components/accounts/AddAccount.tsx
--- a/src/components/accounts/AddAccount.tsx
+++ b/src/components/accounts/AddAccount.tsx
 // Added JMAP as third account type option in the AddAccount chooser UI

diff --git a/src/components/accounts/AddJmapAccount.tsx b/src/components/accounts/AddJmapAccount.tsx
new file mode 100644
--- /dev/null
+++ b/src/components/accounts/AddJmapAccount.tsx
 // 3-step account setup:
 // Step 1: Email + password (or bearer token)
 // Step 2: Auto-discover JMAP URL (.well-known/jmap or known providers)
 // Step 3: Test connection and save
 // (Full component code omitted for brevity — standard React form)

diff --git a/src/services/db/accounts.ts b/src/services/db/accounts.ts
 // Added jmap_url: string | null to DbAccount interface

diff --git a/src/services/db/jmapSyncState.test.ts b/src/services/db/jmapSyncState.test.ts
new file mode 100644
diff --git a/src/services/db/jmapSyncState.ts b/src/services/db/jmapSyncState.ts
new file mode 100644
 // upsertJmapSyncState(accountId, type, state)
 // getJmapSyncState(accountId, type) -> { state: string } | null

diff --git a/src/services/db/migrations.ts b/src/services/db/migrations.ts
 // Migration 18: ALTER TABLE accounts ADD COLUMN jmap_url TEXT;
 //               CREATE TABLE jmap_sync_state (account_id, type, state, updated_at)

diff --git a/src/services/email/jmapProvider.ts b/src/services/email/jmapProvider.ts
new file mode 100644
--- /dev/null
+++ b/src/services/email/jmapProvider.ts
@@ -0,0 +1,463 @@
+import type { EmailProvider, EmailFolder, SyncResult } from "./types";
+import type { ParsedMessage } from "../gmail/messageParser";
+import type { JmapClient } from "../jmap/client";
+import type { JmapEmail, JmapMailbox } from "../jmap/types";
+import {
+  mapMailboxToLabel,
+  buildMailboxMap,
+  findMailboxByRole,
+  labelIdToMailboxId,
+} from "../jmap/mailboxMapper";
+import { jmapEmailToParsedMessage } from "../jmap/jmapSync";
+
+const EMAIL_FETCH_PROPERTIES = [
+  "id", "blobId", "threadId", "mailboxIds", "keywords", "size",
+  "receivedAt", "messageId", "inReplyTo", "references", "from",
+  "to", "cc", "bcc", "replyTo", "subject", "sentAt",
+  "hasAttachment", "preview", "bodyStructure", "textBody",
+  "htmlBody", "attachments",
+];
+
+const BODY_PROPERTIES = ["partId", "blobId", "size", "name", "type", "charset", "disposition", "cid"];
+
+export class JmapProvider implements EmailProvider {
+  readonly accountId: string;
+  readonly type = "jmap" as const;
+  private client: JmapClient;
+  private mailboxCache: JmapMailbox[] | null = null;
+
+  constructor(accountId: string, client: JmapClient) {
+    this.accountId = accountId;
+    this.client = client;
+  }
+
+  private async getMailboxes(): Promise<JmapMailbox[]> {
+    if (this.mailboxCache) return this.mailboxCache;
+    const resp = await this.client.mailboxGet();
+    this.mailboxCache = (resp.list ?? []) as JmapMailbox[];
+    return this.mailboxCache;
+  }
+
+  private invalidateMailboxCache(): void {
+    this.mailboxCache = null;
+  }
+
+  private async resolveMailboxId(labelId: string): Promise<string> {
+    const mailboxes = await this.getMailboxes();
+    const id = labelIdToMailboxId(labelId, mailboxes);
+    if (!id) throw new Error(`Cannot resolve label "${labelId}" to JMAP mailbox`);
+    return id;
+  }
+
+  private async getMailboxByRole(role: string): Promise<JmapMailbox> {
+    const mailboxes = await this.getMailboxes();
+    const mb = findMailboxByRole(mailboxes, role);
+    if (!mb) throw new Error(`No mailbox with role "${role}" found`);
+    return mb;
+  }
+
+  async listFolders(): Promise<EmailFolder[]> {
+    const mailboxes = await this.getMailboxes();
+    return mailboxes.map((mb) => {
+      const mapping = mapMailboxToLabel(mb);
+      return {
+        id: mapping.labelId,
+        name: mapping.labelName,
+        path: mb.name,
+        type: mapping.type as "system" | "user",
+        specialUse: mb.role,
+        delimiter: "/",
+        messageCount: mb.totalEmails,
+        unreadCount: mb.unreadEmails,
+      };
+    });
+  }
+
+  async createFolder(name: string, _parentPath?: string): Promise<EmailFolder> {
+    const mailboxes = await this.getMailboxes();
+    let parentId: string | null = null;
+    if (_parentPath) {
+      const parent = mailboxes.find((mb) => mb.name === _parentPath);
+      parentId = parent?.id ?? null;
+    }
+    const resp = await this.client.mailboxSet({ new1: { name, parentId } });
+    this.invalidateMailboxCache();
+    const created = resp.created as Record<string, { id: string }> | undefined;
+    const newId = created?.new1?.id ?? `jmap-new-${Date.now()}`;
+    return {
+      id: `jmap-${newId}`, name, path: name, type: "user",
+      specialUse: null, delimiter: "/", messageCount: 0, unreadCount: 0,
+    };
+  }
+
+  async deleteFolder(path: string): Promise<void> {
+    const mailboxId = await this.resolveMailboxId(path);
+    await this.client.mailboxSet(undefined, undefined, [mailboxId]);
+    this.invalidateMailboxCache();
+  }
+
+  async renameFolder(path: string, newName: string): Promise<void> {
+    const mailboxId = await this.resolveMailboxId(path);
+    await this.client.mailboxSet(undefined, { [mailboxId]: { name: newName } });
+    this.invalidateMailboxCache();
+  }
+
+  async initialSync(_daysBack: number, _onProgress?: (phase: string, current: number, total: number) => void): Promise<SyncResult> {
+    return { messages: [] }; // Handled by jmapSync.ts module
+  }
+
+  async deltaSync(_syncToken: string): Promise<SyncResult> {
+    return { messages: [] }; // Handled by jmapSync.ts module
+  }
+
+  async fetchMessage(messageId: string): Promise<ParsedMessage> {
+    const resp = await this.client.emailGet([messageId], EMAIL_FETCH_PROPERTIES, BODY_PROPERTIES, true, true);
+    const emails = (resp.list ?? []) as JmapEmail[];
+    if (emails.length === 0) throw new Error(`Email ${messageId} not found`);
+    const mailboxes = await this.getMailboxes();
+    const mailboxMap = buildMailboxMap(mailboxes);
+    return jmapEmailToParsedMessage(emails[0]!, mailboxMap);
+  }
+
+  async fetchAttachment(_messageId: string, attachmentId: string): Promise<{ data: string; size: number }> {
+    const buffer = await this.client.downloadBlob(attachmentId);
+    const bytes = new Uint8Array(buffer);
+    let binary = "";
+    for (const byte of bytes) binary += String.fromCharCode(byte);
+    return { data: btoa(binary), size: bytes.length };
+  }
+
+  async archive(_threadId: string, messageIds: string[]): Promise<void> {
+    const inboxMb = await this.getMailboxByRole("inbox");
+    const archiveMb = findMailboxByRole(await this.getMailboxes(), "archive");
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of messageIds) {
+      const patch: Record<string, unknown> = { [`mailboxIds/${inboxMb.id}`]: null };
+      if (archiveMb) patch[`mailboxIds/${archiveMb.id}`] = true;
+      update[id] = patch;
+    }
+    await this.client.emailSet(undefined, update);
+  }
+
+  async trash(_threadId: string, messageIds: string[]): Promise<void> {
+    const trashMb = await this.getMailboxByRole("trash");
+    const inboxMb = await this.getMailboxByRole("inbox");
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of messageIds) {
+      update[id] = { [`mailboxIds/${trashMb.id}`]: true, [`mailboxIds/${inboxMb.id}`]: null };
+    }
+    await this.client.emailSet(undefined, update);
+  }
+
+  async permanentDelete(_threadId: string, messageIds: string[]): Promise<void> {
+    await this.client.emailSet(undefined, undefined, messageIds);
+  }
+
+  async markRead(_threadId: string, messageIds: string[], read: boolean): Promise<void> {
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of messageIds) update[id] = { "keywords/$seen": read ? true : null };
+    await this.client.emailSet(undefined, update);
+  }
+
+  async star(_threadId: string, messageIds: string[], starred: boolean): Promise<void> {
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of messageIds) update[id] = { "keywords/$flagged": starred ? true : null };
+    await this.client.emailSet(undefined, update);
+  }
+
+  async spam(_threadId: string, messageIds: string[], isSpam: boolean): Promise<void> {
+    const junkMb = await this.getMailboxByRole("junk");
+    const inboxMb = await this.getMailboxByRole("inbox");
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of messageIds) {
+      if (isSpam) {
+        update[id] = { [`mailboxIds/${junkMb.id}`]: true, [`mailboxIds/${inboxMb.id}`]: null };
+      } else {
+        update[id] = { [`mailboxIds/${inboxMb.id}`]: true, [`mailboxIds/${junkMb.id}`]: null };
+      }
+    }
+    await this.client.emailSet(undefined, update);
+  }
+
+  async moveToFolder(_threadId: string, messageIds: string[], folderPath: string): Promise<void> {
+    const targetMailboxId = await this.resolveMailboxId(folderPath);
+    const resp = await this.client.emailGet(messageIds, ["mailboxIds"]);
+    const emails = (resp.list ?? []) as { id: string; mailboxIds: Record<string, boolean> }[];
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const email of emails) {
+      const patch: Record<string, unknown> = { [`mailboxIds/${targetMailboxId}`]: true };
+      for (const mbId of Object.keys(email.mailboxIds)) {
+        if (mbId !== targetMailboxId) patch[`mailboxIds/${mbId}`] = null;
+      }
+      update[email.id] = patch;
+    }
+    await this.client.emailSet(undefined, update);
+  }
+
+  async addLabel(_threadId: string, labelId: string): Promise<void> {
+    const mailboxId = await this.resolveMailboxId(labelId);
+    const queryResp = await this.client.emailQuery({ inThread: _threadId });
+    const ids = (queryResp.ids ?? []) as string[];
+    if (ids.length === 0) return;
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of ids) update[id] = { [`mailboxIds/${mailboxId}`]: true };
+    await this.client.emailSet(undefined, update);
+  }
+
+  async removeLabel(_threadId: string, labelId: string): Promise<void> {
+    const mailboxId = await this.resolveMailboxId(labelId);
+    const queryResp = await this.client.emailQuery({ inThread: _threadId });
+    const ids = (queryResp.ids ?? []) as string[];
+    if (ids.length === 0) return;
+    const update: Record<string, Record<string, unknown>> = {};
+    for (const id of ids) update[id] = { [`mailboxIds/${mailboxId}`]: null };
+    await this.client.emailSet(undefined, update);
+  }
+
+  async sendMessage(rawBase64Url: string, _threadId?: string): Promise<{ id: string }> {
+    const base64 = rawBase64Url.replace(/-/g, "+").replace(/_/g, "/");
+    const binary = atob(base64);
+    const bytes = new Uint8Array(binary.length);
+    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
+    const blobId = await this.client.uploadBlob(bytes, "message/rfc822");
+    const accountId = await this.client.getJmapAccountId();
+    const resp = await this.client.apiCall([
+      ["Email/import", {
+        accountId,
+        emails: { draft1: { blobId, mailboxIds: {}, keywords: {} } },
+      }, "imp0"],
+      ["EmailSubmission/set", {
+        accountId,
+        create: { sub1: { emailId: "#draft1", envelope: null } },
+        onSuccessUpdateEmail: { "#sub1": { "keywords/$draft": null, "keywords/$seen": true } },
+      }, "sub0"],
+    ]);
+    const importResp = this.client.getMethodResponse(resp, "imp0");
+    const created = importResp.created as Record<string, { id: string }> | undefined;
+    return { id: created?.draft1?.id ?? `jmap-sent-${Date.now()}` };
+  }
+
+  async createDraft(rawBase64Url: string, _threadId?: string): Promise<{ draftId: string }> {
+    const base64 = rawBase64Url.replace(/-/g, "+").replace(/_/g, "/");
+    const binary = atob(base64);
+    const bytes = new Uint8Array(binary.length);
+    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
+    const blobId = await this.client.uploadBlob(bytes, "message/rfc822");
+    const draftsMb = await this.getMailboxByRole("drafts");
+    const accountId = await this.client.getJmapAccountId();
+    const resp = await this.client.apiCall([
+      ["Email/import", {
+        accountId,
+        emails: { draft1: { blobId, mailboxIds: { [draftsMb.id]: true }, keywords: { $draft: true, $seen: true } } },
+      }, "imp0"],
+    ]);
+    const importResp = this.client.getMethodResponse(resp, "imp0");
+    const created = importResp.created as Record<string, { id: string }> | undefined;
+    return { draftId: created?.draft1?.id ?? `jmap-draft-${Date.now()}` };
+  }
+
+  async updateDraft(draftId: string, rawBase64Url: string, _threadId?: string): Promise<{ draftId: string }> {
+    await this.deleteDraft(draftId);
+    return this.createDraft(rawBase64Url, _threadId);
+  }
+
+  async deleteDraft(draftId: string): Promise<void> {
+    await this.client.emailSet(undefined, undefined, [draftId]);
+  }
+
+  async testConnection(): Promise<{ success: boolean; message: string }> {
+    return this.client.testConnection();
+  }
+
+  async getProfile(): Promise<{ email: string; name?: string }> {
+    const session = await this.client.getSession();
+    return { email: session.username };
+  }
+}

diff --git a/src/services/email/providerFactory.ts b/src/services/email/providerFactory.ts
--- a/src/services/email/providerFactory.ts
+++ b/src/services/email/providerFactory.ts
+import { JmapProvider } from "./jmapProvider";
+import { createJmapClientForAccount } from "../jmap/clientFactory";

   if (account.provider === "imap") {
     provider = new ImapSmtpProvider(accountId);
+  } else if (account.provider === "jmap") {
+    const client = await createJmapClientForAccount(account);
+    provider = new JmapProvider(accountId, client);
   } else {
     // Default: gmail_api

diff --git a/src/services/email/types.ts b/src/services/email/types.ts
-export type AccountProvider = "gmail_api" | "imap";
+export type AccountProvider = "gmail_api" | "imap" | "jmap";

diff --git a/src/services/jmap/types.ts b/src/services/jmap/types.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/types.ts
@@ -0,0 +1,128 @@
+export type JmapAuthMethod = "basic" | "bearer";
+
+export interface JmapSession {
+  capabilities: Record<string, unknown>;
+  accounts: Record<string, {
+    name: string;
+    isPersonal: boolean;
+    isReadOnly: boolean;
+    accountCapabilities: Record<string, unknown>;
+  }>;
+  primaryAccounts: Record<string, string>;
+  username: string;
+  apiUrl: string;
+  downloadUrl: string;
+  uploadUrl: string;
+  eventSourceUrl: string;
+  state: string;
+}
+
+export interface JmapEmailAddress {
+  name: string | null;
+  email: string;
+}
+
+export interface JmapBodyPart {
+  partId: string | null;
+  blobId: string | null;
+  size: number;
+  name: string | null;
+  type: string;
+  charset: string | null;
+  disposition: string | null;
+  cid: string | null;
+  subParts: JmapBodyPart[] | null;
+}
+
+export interface JmapMailbox {
+  id: string;
+  name: string;
+  parentId: string | null;
+  role: string | null;
+  sortOrder: number;
+  totalEmails: number;
+  unreadEmails: number;
+  totalThreads: number;
+  unreadThreads: number;
+  myRights: {
+    mayReadItems: boolean;
+    mayAddItems: boolean;
+    mayRemoveItems: boolean;
+    maySetSeen: boolean;
+    maySetKeywords: boolean;
+    mayCreateChild: boolean;
+    mayRename: boolean;
+    mayDelete: boolean;
+    maySubmit: boolean;
+  };
+  isSubscribed: boolean;
+}
+
+export interface JmapEmail {
+  id: string;
+  blobId: string;
+  threadId: string;
+  mailboxIds: Record<string, boolean>;
+  keywords: Record<string, boolean>;
+  size: number;
+  receivedAt: string;
+  messageId: string[] | null;
+  inReplyTo: string[] | null;
+  references: string[] | null;
+  sender: JmapEmailAddress[] | null;
+  from: JmapEmailAddress[] | null;
+  to: JmapEmailAddress[] | null;
+  cc: JmapEmailAddress[] | null;
+  bcc: JmapEmailAddress[] | null;
+  replyTo: JmapEmailAddress[] | null;
+  subject: string | null;
+  sentAt: string | null;
+  hasAttachment: boolean;
+  preview: string;
+  bodyStructure: JmapBodyPart | null;
+  bodyValues: Record<string, { value: string; isEncodingProblem: boolean; isTruncated: boolean }> | null;
+  textBody: JmapBodyPart[] | null;
+  htmlBody: JmapBodyPart[] | null;
+  attachments: JmapBodyPart[] | null;
+}
+
+export type JmapMethodCall = [string, Record<string, unknown>, string];
+
+export interface JmapRequest {
+  using: string[];
+  methodCalls: JmapMethodCall[];
+}
+
+export interface JmapResponse {
+  methodResponses: [string, Record<string, unknown>, string][];
+  sessionState: string;
+}
+
+export interface JmapChangesResponse {
+  accountId: string;
+  oldState: string;
+  newState: string;
+  hasMoreChanges: boolean;
+  created: string[];
+  updated: string[];
+  destroyed: string[];
+}
+
+export interface JmapError {
+  type: string;
+  description?: string;
+}

diff --git a/src/services/jmap/client.ts b/src/services/jmap/client.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/client.ts
@@ -0,0 +1,336 @@
+import { fetch } from "@tauri-apps/plugin-http";
+import type {
+  JmapSession, JmapRequest, JmapResponse, JmapMethodCall, JmapAuthMethod,
+} from "./types";
+
+const JMAP_CORE_CAPABILITY = "urn:ietf:params:jmap:core";
+const JMAP_MAIL_CAPABILITY = "urn:ietf:params:jmap:mail";
+const JMAP_SUBMISSION_CAPABILITY = "urn:ietf:params:jmap:submission";
+
+export class JmapClient {
+  private sessionUrl: string;
+  private authMethod: JmapAuthMethod;
+  private authCredential: string;
+  private session: JmapSession | null = null;
+  private accountId: string | null = null;
+
+  constructor(sessionUrl: string, authMethod: JmapAuthMethod, authCredential: string) {
+    this.sessionUrl = sessionUrl;
+    this.authMethod = authMethod;
+    this.authCredential = authCredential;
+  }
+
+  private getAuthHeader(): string {
+    if (this.authMethod === "basic") return `Basic ${this.authCredential}`;
+    return `Bearer ${this.authCredential}`;
+  }
+
+  async getSession(): Promise<JmapSession> {
+    if (this.session) return this.session;
+    const resp = await fetch(this.sessionUrl, {
+      method: "GET",
+      headers: { Authorization: this.getAuthHeader(), Accept: "application/json" },
+    });
+    if (!resp.ok) throw new Error(`JMAP session discovery failed: ${resp.status} ${resp.statusText}`);
+    this.session = (await resp.json()) as JmapSession;
+    this.accountId =
+      this.session.primaryAccounts[JMAP_MAIL_CAPABILITY] ??
+      Object.keys(this.session.accounts)[0] ?? null;
+    return this.session;
+  }
+
+  clearSession(): void { this.session = null; this.accountId = null; }
+
+  async getJmapAccountId(): Promise<string> {
+    if (!this.accountId) await this.getSession();
+    if (!this.accountId) throw new Error("No JMAP mail account found in session");
+    return this.accountId;
+  }
+
+  async apiCall(methodCalls: JmapMethodCall[]): Promise<JmapResponse> {
+    const session = await this.getSession();
+    const request: JmapRequest = {
+      using: [JMAP_CORE_CAPABILITY, JMAP_MAIL_CAPABILITY, JMAP_SUBMISSION_CAPABILITY],
+      methodCalls,
+    };
+    const resp = await fetch(session.apiUrl, {
+      method: "POST",
+      headers: {
+        Authorization: this.getAuthHeader(),
+        "Content-Type": "application/json",
+        Accept: "application/json",
+      },
+      body: JSON.stringify(request),
+    });
+    if (!resp.ok) throw new Error(`JMAP API call failed: ${resp.status} ${resp.statusText}`);
+    const response = (await resp.json()) as JmapResponse;
+    if (response.sessionState !== session.state) this.clearSession();
+    return response;
+  }
+
+  getMethodResponse(response: JmapResponse, callId: string): Record<string, unknown> {
+    for (const [name, args, id] of response.methodResponses) {
+      if (id === callId) {
+        if (name === "error") {
+          const errType = (args as Record<string, unknown>).type ?? "unknown";
+          const errDesc = (args as Record<string, unknown>).description ?? "";
+          throw new Error(`JMAP error (${errType}): ${errDesc}`);
+        }
+        return args;
+      }
+    }
+    throw new Error(`No response found for call ID: ${callId}`);
+  }
+
+  async mailboxGet(properties?: string[]): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const args: Record<string, unknown> = { accountId };
+    if (properties) args.properties = properties;
+    const resp = await this.apiCall([["Mailbox/get", args, "mb0"]]);
+    return this.getMethodResponse(resp, "mb0");
+  }
+
+  async mailboxChanges(sinceState: string): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const resp = await this.apiCall([["Mailbox/changes", { accountId, sinceState }, "mbc0"]]);
+    return this.getMethodResponse(resp, "mbc0");
+  }
+
+  async mailboxSet(
+    create?: Record<string, unknown>,
+    update?: Record<string, Record<string, unknown>>,
+    destroy?: string[],
+  ): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const args: Record<string, unknown> = { accountId };
+    if (create) args.create = create;
+    if (update) args.update = update;
+    if (destroy) args.destroy = destroy;
+    const resp = await this.apiCall([["Mailbox/set", args, "mbs0"]]);
+    return this.getMethodResponse(resp, "mbs0");
+  }
+
+  async emailQuery(
+    filter: Record<string, unknown>,
+    sort?: Record<string, unknown>[],
+    position?: number,
+    limit?: number,
+  ): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const args: Record<string, unknown> = { accountId, filter };
+    if (sort) args.sort = sort;
+    if (position !== undefined) args.position = position;
+    if (limit !== undefined) args.limit = limit;
+    const resp = await this.apiCall([["Email/query", args, "eq0"]]);
+    return this.getMethodResponse(resp, "eq0");
+  }
+
+  async emailGet(
+    ids: string[],
+    properties?: string[],
+    bodyProperties?: string[],
+    fetchTextBodyValues?: boolean,
+    fetchHTMLBodyValues?: boolean,
+  ): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const args: Record<string, unknown> = { accountId, ids };
+    if (properties) args.properties = properties;
+    if (bodyProperties) args.bodyProperties = bodyProperties;
+    if (fetchTextBodyValues) args.fetchTextBodyValues = true;
+    if (fetchHTMLBodyValues) args.fetchHTMLBodyValues = true;
+    const resp = await this.apiCall([["Email/get", args, "eg0"]]);
+    return this.getMethodResponse(resp, "eg0");
+  }
+
+  async emailChanges(sinceState: string): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const resp = await this.apiCall([["Email/changes", { accountId, sinceState }, "ec0"]]);
+    return this.getMethodResponse(resp, "ec0");
+  }
+
+  async emailSet(
+    create?: Record<string, unknown>,
+    update?: Record<string, Record<string, unknown>>,
+    destroy?: string[],
+  ): Promise<Record<string, unknown>> {
+    const accountId = await this.getJmapAccountId();
+    const args: Record<string, unknown> = { accountId };
+    if (create) args.create = create;
+    if (update) args.update = update;
+    if (destroy) args.destroy = destroy;
+    const resp = await this.apiCall([["Email/set", args, "es0"]]);
+    return this.getMethodResponse(resp, "es0");
+  }
+
+  async downloadBlob(blobId: string): Promise<ArrayBuffer> {
+    const session = await this.getSession();
+    const accountId = await this.getJmapAccountId();
+    const url = session.downloadUrl
+      .replace("{accountId}", encodeURIComponent(accountId))
+      .replace("{blobId}", encodeURIComponent(blobId))
+      .replace("{type}", "application/octet-stream")
+      .replace("{name}", "download");
+    const resp = await fetch(url, {
+      method: "GET",
+      headers: { Authorization: this.getAuthHeader() },
+    });
+    if (!resp.ok) throw new Error(`Blob download failed: ${resp.status}`);
+    return resp.arrayBuffer();
+  }
+
+  async uploadBlob(data: Uint8Array, type: string): Promise<string> {
+    const session = await this.getSession();
+    const accountId = await this.getJmapAccountId();
+    const url = session.uploadUrl.replace("{accountId}", encodeURIComponent(accountId));
+    const resp = await fetch(url, {
+      method: "POST",
+      headers: { Authorization: this.getAuthHeader(), "Content-Type": type },
+      body: data.buffer as ArrayBuffer,
+    });
+    if (!resp.ok) throw new Error(`Blob upload failed: ${resp.status}`);
+    const result = (await resp.json()) as { blobId: string; type: string; size: number };
+    return result.blobId;
+  }
+
+  async testConnection(): Promise<{ success: boolean; message: string }> {
+    try {
+      const session = await this.getSession();
+      if (!(JMAP_MAIL_CAPABILITY in session.capabilities)) {
+        return { success: false, message: "Server does not support JMAP Mail (urn:ietf:params:jmap:mail)" };
+      }
+      return { success: true, message: `Connected as ${session.username}` };
+    } catch (err) {
+      return { success: false, message: err instanceof Error ? err.message : "Unknown connection error" };
+    }
+  }
+}

diff --git a/src/services/jmap/clientFactory.ts b/src/services/jmap/clientFactory.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/clientFactory.ts
@@ -0,0 +1,33 @@
+import { JmapClient } from "./client";
+import type { DbAccount } from "../db/accounts";
+import type { JmapAuthMethod } from "./types";
+
+export async function createJmapClientForAccount(account: DbAccount): Promise<JmapClient> {
+  if (!account.jmap_url) throw new Error("JMAP URL not configured for this account");
+  let authMethod: JmapAuthMethod;
+  let authCredential: string;
+  if (account.auth_method === "oauth2" || account.auth_method === "bearer") {
+    authMethod = "bearer";
+    if (!account.access_token) throw new Error("No access token available for JMAP account");
+    authCredential = account.access_token;
+  } else {
+    authMethod = "basic";
+    const password = account.imap_password ?? "";
+    authCredential = btoa(`${account.email}:${password}`);
+  }
+  return new JmapClient(account.jmap_url, authMethod, authCredential);
+}

diff --git a/src/services/jmap/autoDiscovery.ts b/src/services/jmap/autoDiscovery.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/autoDiscovery.ts
@@ -0,0 +1,58 @@
+import { fetch } from "@tauri-apps/plugin-http";
+
+const KNOWN_PROVIDERS: Record<string, string> = {
+  "fastmail.com": "https://api.fastmail.com/jmap/session",
+  "messagingengine.com": "https://api.fastmail.com/jmap/session",
+};
+
+export interface JmapDiscoveryResult {
+  sessionUrl: string;
+  source: "well-known" | "known-provider" | "manual";
+}
+
+export async function discoverJmapUrl(email: string): Promise<JmapDiscoveryResult | null> {
+  const domain = email.split("@")[1]?.toLowerCase();
+  if (!domain) return null;
+  const knownUrl = KNOWN_PROVIDERS[domain];
+  if (knownUrl) return { sessionUrl: knownUrl, source: "known-provider" };
+  try {
+    const wellKnownUrl = `https://${domain}/.well-known/jmap`;
+    const resp = await fetch(wellKnownUrl, { method: "GET", headers: { Accept: "application/json" } });
+    if (resp.ok) return { sessionUrl: wellKnownUrl, source: "well-known" };
+  } catch { /* Discovery failed */ }
+  return null;
+}
+
+export function isKnownJmapProvider(email: string): boolean {
+  const domain = email.split("@")[1]?.toLowerCase();
+  return !!domain && domain in KNOWN_PROVIDERS;
+}

diff --git a/src/services/jmap/mailboxMapper.ts b/src/services/jmap/mailboxMapper.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/mailboxMapper.ts
@@ -0,0 +1,145 @@
+import type { JmapMailbox } from "./types";
+import { upsertLabel } from "../db/labels";
+
+const ROLE_MAP: Record<string, { labelId: string; labelName: string; type: string }> = {
+  inbox:     { labelId: "INBOX",     labelName: "Inbox",     type: "system" },
+  archive:   { labelId: "archive",   labelName: "Archive",   type: "system" },
+  drafts:    { labelId: "DRAFT",     labelName: "Drafts",    type: "system" },
+  sent:      { labelId: "SENT",      labelName: "Sent",      type: "system" },
+  trash:     { labelId: "TRASH",     labelName: "Trash",     type: "system" },
+  junk:      { labelId: "SPAM",      labelName: "Spam",      type: "system" },
+  important: { labelId: "IMPORTANT", labelName: "Important", type: "system" },
+};
+
+export interface MailboxLabelMapping {
+  labelId: string;
+  labelName: string;
+  type: string;
+}
+
+export function mapMailboxToLabel(mailbox: JmapMailbox): MailboxLabelMapping {
+  if (mailbox.role) {
+    const mapping = ROLE_MAP[mailbox.role];
+    if (mapping) return mapping;
+  }
+  return { labelId: `jmap-${mailbox.id}`, labelName: mailbox.name, type: "user" };
+}
+
+export function getLabelsForJmapEmail(
+  mailboxIds: Record<string, boolean>,
+  keywords: Record<string, boolean>,
+  mailboxMap: Map<string, JmapMailbox>,
+): string[] {
+  const labels: string[] = [];
+  for (const mailboxId of Object.keys(mailboxIds)) {
+    const mailbox = mailboxMap.get(mailboxId);
+    if (mailbox) labels.push(mapMailboxToLabel(mailbox).labelId);
+  }
+  if (!keywords["$seen"]) labels.push("UNREAD");
+  if (keywords["$flagged"]) labels.push("STARRED");
+  if (keywords["$draft"] && !labels.includes("DRAFT")) labels.push("DRAFT");
+  return labels;
+}
+
+export async function syncMailboxesToLabels(accountId: string, mailboxes: JmapMailbox[]): Promise<void> {
+  for (const mailbox of mailboxes) {
+    const mapping = mapMailboxToLabel(mailbox);
+    await upsertLabel({ id: mapping.labelId, accountId, name: mapping.labelName, type: mapping.type });
+  }
+  await upsertLabel({ id: "UNREAD", accountId, name: "Unread", type: "system" });
+}
+
+export function buildMailboxMap(mailboxes: JmapMailbox[]): Map<string, JmapMailbox> {
+  const map = new Map<string, JmapMailbox>();
+  for (const mb of mailboxes) map.set(mb.id, mb);
+  return map;
+}
+
+export function findMailboxByRole(mailboxes: JmapMailbox[], role: string): JmapMailbox | undefined {
+  return mailboxes.find((mb) => mb.role === role);
+}
+
+export function labelIdToMailboxId(labelId: string, mailboxes: JmapMailbox[]): string | null {
+  for (const [role, mapping] of Object.entries(ROLE_MAP)) {
+    if (mapping.labelId === labelId) {
+      const mb = findMailboxByRole(mailboxes, role);
+      return mb?.id ?? null;
+    }
+  }
+  if (labelId.startsWith("jmap-")) return labelId.slice(5);
+  return null;
+}

diff --git a/src/services/jmap/jmapSync.ts b/src/services/jmap/jmapSync.ts
new file mode 100644
--- /dev/null
+++ b/src/services/jmap/jmapSync.ts
@@ -0,0 +1,386 @@
+import type { JmapClient } from "./client";
+import type { JmapEmail, JmapMailbox } from "./types";
+import type { ParsedMessage, ParsedAttachment } from "../gmail/messageParser";
+import { upsertMessage } from "../db/messages";
+import { upsertThread, setThreadLabels } from "../db/threads";
+import { upsertAttachment } from "../db/attachments";
+import { updateAccountSyncState } from "../db/accounts";
+import { upsertJmapSyncState, getJmapSyncState } from "../db/jmapSyncState";
+import { syncMailboxesToLabels, buildMailboxMap, getLabelsForJmapEmail } from "./mailboxMapper";
+import { upsertContact } from "../db/contacts";
+
+const EMAIL_PROPERTIES = [
+  "id", "blobId", "threadId", "mailboxIds", "keywords", "size",
+  "receivedAt", "messageId", "inReplyTo", "references", "from",
+  "to", "cc", "bcc", "replyTo", "subject", "sentAt",
+  "hasAttachment", "preview", "bodyStructure", "textBody",
+  "htmlBody", "attachments",
+];
+const BODY_PROPERTIES = ["partId", "blobId", "size", "name", "type", "charset", "disposition", "cid"];
+const BATCH_SIZE = 50;
+
+function formatAddresses(
+  addrs: { name: string | null; email: string }[] | null | undefined,
+): string | null {
+  if (!addrs || addrs.length === 0) return null;
+  return addrs.map((a) => (a.name ? `${a.name} <${a.email}>` : a.email)).join(", ");
+}
+
+export function jmapEmailToParsedMessage(
+  email: JmapEmail,
+  mailboxMap: Map<string, JmapMailbox>,
+): ParsedMessage {
+  const from = email.from?.[0];
+  const isRead = !!email.keywords["$seen"];
+  const isStarred = !!email.keywords["$flagged"];
+  const date = email.sentAt ? new Date(email.sentAt).getTime() : new Date(email.receivedAt).getTime();
+  const internalDate = new Date(email.receivedAt).getTime();
+  const labelIds = getLabelsForJmapEmail(email.mailboxIds, email.keywords, mailboxMap);
+
+  let bodyHtml: string | null = null;
+  let bodyText: string | null = null;
+  if (email.bodyValues) {
+    if (email.htmlBody?.[0]?.partId) {
+      const val = email.bodyValues[email.htmlBody[0].partId];
+      if (val) bodyHtml = val.value;
+    }
+    if (email.textBody?.[0]?.partId) {
+      const val = email.bodyValues[email.textBody[0].partId];
+      if (val) bodyText = val.value;
+    }
+  }
+
+  const attachments: ParsedAttachment[] = (email.attachments ?? []).map((att) => ({
+    filename: att.name ?? "attachment",
+    mimeType: att.type,
+    size: att.size,
+    gmailAttachmentId: att.blobId ?? "",
+    contentId: att.cid,
+    isInline: att.disposition === "inline",
+  }));
+
+  return {
+    id: email.id, threadId: email.threadId,
+    fromAddress: from?.email ?? null, fromName: from?.name ?? null,
+    toAddresses: formatAddresses(email.to),
+    ccAddresses: formatAddresses(email.cc),
+    bccAddresses: formatAddresses(email.bcc),
+    replyTo: formatAddresses(email.replyTo),
+    subject: email.subject, snippet: email.preview,
+    date, isRead, isStarred, bodyHtml, bodyText,
+    rawSize: email.size, internalDate, labelIds,
+    hasAttachments: email.hasAttachment, attachments,
+    listUnsubscribe: null, listUnsubscribePost: null, authResults: null,
+  };
+}
+
+async function persistMessages(accountId: string, messages: ParsedMessage[]): Promise<void> {
+  for (const parsed of messages) {
+    await upsertMessage({
+      id: parsed.id, accountId, threadId: parsed.threadId,
+      fromAddress: parsed.fromAddress, fromName: parsed.fromName,
+      toAddresses: parsed.toAddresses, ccAddresses: parsed.ccAddresses,
+      bccAddresses: parsed.bccAddresses, replyTo: parsed.replyTo,
+      subject: parsed.subject, snippet: parsed.snippet,
+      date: parsed.date, isRead: parsed.isRead, isStarred: parsed.isStarred,
+      bodyHtml: parsed.bodyHtml, bodyText: parsed.bodyText,
+      rawSize: parsed.rawSize, internalDate: parsed.internalDate,
+      messageIdHeader: null, referencesHeader: null, inReplyToHeader: null,
+    });
+    await upsertThread({
+      id: parsed.threadId, accountId, subject: parsed.subject,
+      snippet: parsed.snippet, lastMessageAt: parsed.date,
+      messageCount: 1, isRead: parsed.isRead, isStarred: parsed.isStarred,
+      isImportant: parsed.labelIds.includes("IMPORTANT"),
+      hasAttachments: parsed.hasAttachments,
+    });
+    await setThreadLabels(accountId, parsed.threadId, parsed.labelIds);
+    for (const att of parsed.attachments) {
+      await upsertAttachment({
+        id: `${parsed.id}-${att.gmailAttachmentId}`,
+        messageId: parsed.id, accountId, filename: att.filename,
+        mimeType: att.mimeType, size: att.size,
+        gmailAttachmentId: att.gmailAttachmentId,
+        contentId: att.contentId, isInline: att.isInline,
+      });
+    }
+    if (parsed.fromAddress) await upsertContact(parsed.fromAddress, parsed.fromName);
+  }
+}
+
+export interface JmapSyncProgress {
+  phase: "mailboxes" | "messages" | "done";
+  current: number;
+  total: number;
+}
+
+export async function jmapInitialSync(
+  client: JmapClient, accountId: string, daysBack: number,
+  onProgress?: (progress: JmapSyncProgress) => void,
+): Promise<void> {
+  onProgress?.({ phase: "mailboxes", current: 0, total: 1 });
+  const mbResp = await client.mailboxGet();
+  const mailboxes = (mbResp.list ?? []) as JmapMailbox[];
+  const mailboxState = mbResp.state as string;
+  const mailboxMap = buildMailboxMap(mailboxes);
+  await syncMailboxesToLabels(accountId, mailboxes);
+  await upsertJmapSyncState(accountId, "Mailbox", mailboxState);
+  onProgress?.({ phase: "mailboxes", current: 1, total: 1 });
+
+  const sinceDate = new Date();
+  sinceDate.setDate(sinceDate.getDate() - daysBack);
+  const sinceIso = sinceDate.toISOString();
+
+  const countResp = await client.emailQuery(
+    { after: sinceIso }, [{ property: "receivedAt", isAscending: false }], 0, 0,
+  );
+  const totalEmails = (countResp.total ?? 0) as number;
+
+  let position = 0;
+  let fetched = 0;
+  while (position < totalEmails || position === 0) {
+    onProgress?.({ phase: "messages", current: fetched, total: totalEmails });
+    const queryResp = await client.emailQuery(
+      { after: sinceIso }, [{ property: "receivedAt", isAscending: false }], position, BATCH_SIZE,
+    );
+    const emailIds = (queryResp.ids ?? []) as string[];
+    if (emailIds.length === 0) break;
+    const getResp = await client.emailGet(emailIds, EMAIL_PROPERTIES, BODY_PROPERTIES, true, true);
+    const emails = (getResp.list ?? []) as JmapEmail[];
+    const parsed = emails.map((e) => jmapEmailToParsedMessage(e, mailboxMap));
+    await persistMessages(accountId, parsed);
+    fetched += emails.length;
+    position += BATCH_SIZE;
+  }
+
+  const emailStateResp = await client.emailGet([], EMAIL_PROPERTIES.slice(0, 1));
+  const emailState = emailStateResp.state as string;
+  await upsertJmapSyncState(accountId, "Email", emailState);
+  await updateAccountSyncState(accountId, `jmap:${emailState}`);
+  onProgress?.({ phase: "done", current: fetched, total: totalEmails });
+}
+
+export async function jmapDeltaSync(client: JmapClient, accountId: string): Promise<void> {
+  const emailSyncState = await getJmapSyncState(accountId, "Email");
+  const mailboxSyncState = await getJmapSyncState(accountId, "Mailbox");
+  if (!emailSyncState) throw new Error("JMAP_NO_STATE");
+
+  if (mailboxSyncState) {
+    try {
+      const mbChanges = await client.mailboxChanges(mailboxSyncState.state);
+      const newMbState = mbChanges.newState as string;
+      if (newMbState !== mailboxSyncState.state) {
+        const mbResp = await client.mailboxGet();
+        const mailboxes = (mbResp.list ?? []) as JmapMailbox[];
+        await syncMailboxesToLabels(accountId, mailboxes);
+        await upsertJmapSyncState(accountId, "Mailbox", newMbState);
+      }
+    } catch (err) {
+      const msg = err instanceof Error ? err.message : "";
+      if (msg.includes("cannotCalculateChanges")) {
+        const mbResp = await client.mailboxGet();
+        const mailboxes = (mbResp.list ?? []) as JmapMailbox[];
+        const mbState = mbResp.state as string;
+        await syncMailboxesToLabels(accountId, mailboxes);
+        await upsertJmapSyncState(accountId, "Mailbox", mbState);
+      } else { throw err; }
+    }
+  }
+
+  const mbResp = await client.mailboxGet();
+  const mailboxes = (mbResp.list ?? []) as JmapMailbox[];
+  const mailboxMap = buildMailboxMap(mailboxes);
+
+  try {
+    let sinceState = emailSyncState.state;
+    let hasMore = true;
+    while (hasMore) {
+      const changes = await client.emailChanges(sinceState);
+      const created = (changes.created ?? []) as string[];
+      const updated = (changes.updated ?? []) as string[];
+      const destroyed = (changes.destroyed ?? []) as string[];
+      const newState = changes.newState as string;
+      hasMore = (changes.hasMoreChanges ?? false) as boolean;
+
+      const idsToFetch = [...created, ...updated];
+      if (idsToFetch.length > 0) {
+        for (let i = 0; i < idsToFetch.length; i += BATCH_SIZE) {
+          const batch = idsToFetch.slice(i, i + BATCH_SIZE);
+          const getResp = await client.emailGet(batch, EMAIL_PROPERTIES, BODY_PROPERTIES, true, true);
+          const emails = (getResp.list ?? []) as JmapEmail[];
+          const parsed = emails.map((e) => jmapEmailToParsedMessage(e, mailboxMap));
+          await persistMessages(accountId, parsed);
+        }
+      }
+
+      if (destroyed.length > 0) {
+        const { getDb } = await import("../db/connection");
+        const db = await getDb();
+        for (const emailId of destroyed) {
+          await db.execute("DELETE FROM messages WHERE account_id = $1 AND id = $2", [accountId, emailId]);
+        }
+      }
+      sinceState = newState;
+    }
+    await upsertJmapSyncState(accountId, "Email", sinceState);
+    await updateAccountSyncState(accountId, `jmap:${sinceState}`);
+  } catch (err) {
+    const msg = err instanceof Error ? err.message : "";
+    if (msg.includes("cannotCalculateChanges")) throw new Error("JMAP_STATE_EXPIRED");
+    throw err;
+  }
+}

diff --git a/src/services/gmail/syncManager.ts b/src/services/gmail/syncManager.ts
--- a/src/services/gmail/syncManager.ts
+++ b/src/services/gmail/syncManager.ts
+import { jmapInitialSync, jmapDeltaSync } from "../jmap/jmapSync";
+import { createJmapClientForAccount } from "../jmap/clientFactory";
+
+async function syncJmapAccount(accountId: string): Promise<void> {
+  const account = await getAccount(accountId);
+  if (!account) throw new Error("Account not found");
+  const client = await createJmapClientForAccount(account);
+  const syncPeriodStr = await getSetting("sync_period_days");
+  const syncDays = parseInt(syncPeriodStr ?? "365", 10) || 365;
+  if (account.history_id) {
+    try {
+      await jmapDeltaSync(client, accountId);
+    } catch (err) {
+      const message = err instanceof Error ? err.message : "";
+      if (message === "JMAP_STATE_EXPIRED" || message === "JMAP_NO_STATE") {
+        await jmapInitialSync(client, accountId, syncDays, (progress) => {
+          statusCallback?.(accountId, "syncing", {
+            phase: progress.phase === "mailboxes" ? "labels" : progress.phase as "labels" | "threads" | "messages" | "done",
+            current: progress.current, total: progress.total,
+          });
+        });
+      } else { throw err; }
+    }
+  } else {
+    await jmapInitialSync(client, accountId, syncDays, (progress) => {
+      statusCallback?.(accountId, "syncing", {
+        phase: progress.phase === "mailboxes" ? "labels" : progress.phase as "labels" | "threads" | "messages" | "done",
+        current: progress.current, total: progress.total,
+      });
+    });
+  }
+}
+
 // In syncAccountInternal():
+    } else if (account.provider === "jmap") {
+      await syncJmapAccount(accountId);
```

---

### IMAP UTF-7 + Sparse UID Fix (4e4f8f1) — already in our codebase

```diff
diff --git a/src-tauri/Cargo.toml b/src-tauri/Cargo.toml
+utf7-imap = "0.3"

diff --git a/src-tauri/src/imap/types.rs b/src-tauri/src/imap/types.rs
 pub struct ImapFolder {
-    pub path: String,
-    pub name: String,
+    pub path: String,      // decoded UTF-8 display name
+    pub raw_path: String,  // original modified UTF-7 path for IMAP commands
+    pub name: String,      // decoded display name (last segment)

diff --git a/src-tauri/src/imap/client.rs b/src-tauri/src/imap/client.rs
 // list_folders(): decode modified UTF-7 via utf7_imap::decode_utf7_imap()
 // Use raw_path for IMAP STATUS commands
+pub async fn search_all_uids(session: &mut ImapSession, folder: &str) -> Result<Vec<u32>, String> {
+    session.select(folder).await.map_err(|e| format!("SELECT {folder} failed: {e}"))?;
+    let uids = session.uid_search("ALL").await.map_err(|e| format!("UID SEARCH ALL failed: {e}"))?;
+    let mut result: Vec<u32> = uids.into_iter().collect();
+    result.sort();
+    Ok(result)
+}

diff --git a/src/services/imap/imapSync.ts b/src/services/imap/imapSync.ts
 // Removed generateUidRange() heuristic
 // Replaced with imapSearchAllUids(config, folder.raw_path)
 // All IMAP operations now use folder.raw_path instead of folder.path
 // folderMapper.ts: store raw_path as imapFolderPath in labels
```
