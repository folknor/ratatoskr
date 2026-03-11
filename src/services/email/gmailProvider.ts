import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import type { EmailFolder, EmailProvider, SyncResult } from "./types";

/** Map Gmail system label IDs to IMAP special-use flags */
const GMAIL_SPECIAL_USE: Record<string, string | null> = {
  INBOX: null,
  SENT: "\\Sent",
  TRASH: "\\Trash",
  DRAFT: "\\Drafts",
  SPAM: "\\Junk",
  STARRED: null,
  IMPORTANT: null,
  CATEGORY_PERSONAL: null,
  CATEGORY_SOCIAL: null,
  CATEGORY_PROMOTIONS: null,
  CATEGORY_UPDATES: null,
  CATEGORY_FORUMS: null,
  UNREAD: null,
  CHAT: null,
};

/** Shape returned by the Rust gmail_list_labels command */
interface RustGmailLabel {
  id: string;
  name: string;
  labelType: string | null;
  messagesTotal: number | null;
  messagesUnread: number | null;
}

/** Shape returned by the Rust gmail_test_connection command */
interface RustGmailProfile {
  emailAddress: string;
  messagesTotal: number | null;
  threadsTotal: number | null;
  historyId: string;
}

/** Shape returned by the Rust gmail_get_history command */
interface RustGmailHistoryResponse {
  history: RustGmailHistoryItem[];
  historyId: string;
  nextPageToken: string | null;
}

interface RustGmailHistoryItem {
  id: string;
  messagesAdded: { message: { id: string } }[];
  messagesDeleted: { message: { id: string } }[];
  labelsAdded: { message: { id: string }; labelIds: string[] }[];
  labelsRemoved: { message: { id: string }; labelIds: string[] }[];
}

/** Shape returned by the Rust gmail_get_message with format=raw */
interface RustGmailMessage {
  id: string;
  threadId: string;
  raw?: string;
}

/** Shape returned by the Rust gmail_fetch_attachment command */
interface RustGmailAttachmentData {
  attachmentId: string | null;
  size: number | null;
  data: string;
}

interface ProviderFolderResult {
  id: string;
  name: string;
  path: string;
  folderType: string;
  specialUse: string | null;
  colorBg: string | null;
  colorFg: string | null;
}

interface ProviderTestResult {
  success: boolean;
  message: string;
}

interface ProviderProfile {
  email: string;
  name?: string;
}

/**
 * EmailProvider adapter that delegates to Rust Gmail Tauri commands.
 * All operations invoke the corresponding `gmail_*` Tauri command.
 */
export class GmailApiProvider implements EmailProvider {
  readonly accountId: string;
  readonly type = "gmail_api" as const;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  async listFolders(): Promise<EmailFolder[]> {
    const labels = await invoke<RustGmailLabel[]>("gmail_list_labels", {
      accountId: this.accountId,
    });
    return labels.map((label) => ({
      id: label.id,
      name: label.name,
      path: label.id,
      type:
        label.labelType === "system" ? ("system" as const) : ("user" as const),
      specialUse:
        label.labelType === "system"
          ? (GMAIL_SPECIAL_USE[label.id] ?? null)
          : null,
      delimiter: "/",
      messageCount: label.messagesTotal ?? 0,
      unreadCount: label.messagesUnread ?? 0,
    }));
  }

  async createFolder(name: string, _parentPath?: string): Promise<EmailFolder> {
    const folder = await invoke<ProviderFolderResult>(
      "provider_create_folder",
      {
        accountId: this.accountId,
        name,
        parentId: _parentPath ?? null,
        textColor: null,
        bgColor: null,
      },
    );
    return {
      id: folder.id,
      name: folder.name,
      path: folder.path,
      type: folder.folderType === "system" ? "system" : "user",
      specialUse: folder.specialUse,
      delimiter: "/",
      messageCount: 0,
      unreadCount: 0,
    };
  }

  async deleteFolder(path: string): Promise<void> {
    await invoke<void>("provider_delete_folder", {
      accountId: this.accountId,
      folderId: path,
    });
  }

  async renameFolder(path: string, newName: string): Promise<void> {
    await invoke<ProviderFolderResult>("provider_rename_folder", {
      accountId: this.accountId,
      folderId: path,
      newName,
      textColor: null,
      bgColor: null,
    });
  }

  async initialSync(
    _daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    // Initial sync is handled by the existing sync.ts module.
    // This is a thin wrapper that returns the interface-compatible result.
    // Full integration will wire this up to the existing initialSync function.
    const profile = await invoke<RustGmailProfile>("gmail_test_connection", {
      accountId: this.accountId,
    });
    return {
      messages: [],
      latestSyncToken: profile.historyId,
    };
  }

  async deltaSync(syncToken: string): Promise<SyncResult> {
    // Delta sync is handled by the existing sync.ts module.
    // This is a thin wrapper that returns the interface-compatible result.
    const allMessages: ParsedMessage[] = [];
    let pageToken: string | undefined;
    let latestHistoryId = syncToken;

    do {
      const resp = await invoke<RustGmailHistoryResponse>("gmail_get_history", {
        accountId: this.accountId,
        startHistoryId: syncToken,
        pageToken: pageToken ?? null,
      });
      latestHistoryId = resp.historyId;

      if (resp.history) {
        for (const item of resp.history) {
          if (item.messagesAdded) {
            for (const added of item.messagesAdded) {
              const full = await invoke<ParsedMessage>(
                "gmail_get_parsed_message",
                {
                  accountId: this.accountId,
                  messageId: added.message.id,
                },
              );
              allMessages.push(full);
            }
          }
        }
      }

      pageToken = resp.nextPageToken ?? undefined;
    } while (pageToken);

    return {
      messages: allMessages,
      latestSyncToken: latestHistoryId,
    };
  }

  async fetchMessage(messageId: string): Promise<ParsedMessage> {
    return invoke<ParsedMessage>("gmail_get_parsed_message", {
      accountId: this.accountId,
      messageId,
    });
  }

  async fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }> {
    const resp = await invoke<RustGmailAttachmentData>(
      "gmail_fetch_attachment",
      {
        accountId: this.accountId,
        messageId,
        attachmentId,
      },
    );
    return { data: resp.data, size: resp.size ?? 0 };
  }

  async fetchRawMessage(messageId: string): Promise<string> {
    // Gmail API with format=raw returns a { raw: string } field (base64url-encoded RFC822)
    const resp = await invoke<RustGmailMessage>("gmail_get_message", {
      accountId: this.accountId,
      messageId,
      format: "raw",
    });
    const raw = resp.raw ?? "";
    const base64 = raw.replace(/-/g, "+").replace(/_/g, "/");
    return atob(base64);
  }

  async testConnection(): Promise<{ success: boolean; message: string }> {
    return invoke<ProviderTestResult>("provider_test_connection", {
      accountId: this.accountId,
    });
  }

  async getProfile(): Promise<{ email: string; name?: string | undefined }> {
    return invoke<ProviderProfile>("provider_get_profile", {
      accountId: this.accountId,
    });
  }
}
