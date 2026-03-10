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

/** Shape returned by the Rust gmail_send_email command */
interface RustGmailSendResult {
  id: string;
}

/** Shape returned by the Rust gmail_create_draft / gmail_update_draft commands */
interface RustGmailDraft {
  id: string;
  message: { id: string; threadId: string };
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
      path: label.name,
      type: label.labelType === "system" ? ("system" as const) : ("user" as const),
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
    const fullName = _parentPath ? `${_parentPath}/${name}` : name;
    const label = await invoke<RustGmailLabel>("gmail_create_label", {
      accountId: this.accountId,
      name: fullName,
    });
    return {
      id: label.id,
      name: label.name,
      path: label.name,
      type: "user",
      specialUse: null,
      delimiter: "/",
      messageCount: 0,
      unreadCount: 0,
    };
  }

  async deleteFolder(path: string): Promise<void> {
    // In Gmail, path is the label ID for deletion
    await invoke<void>("gmail_delete_label", {
      accountId: this.accountId,
      labelId: path,
    });
  }

  async renameFolder(path: string, newName: string): Promise<void> {
    await invoke<RustGmailLabel>("gmail_update_label", {
      accountId: this.accountId,
      labelId: path,
      name: newName,
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

  async archive(threadId: string, _messageIds: string[]): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: [],
      removeLabels: ["INBOX"],
    });
  }

  async trash(threadId: string, _messageIds: string[]): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: ["TRASH"],
      removeLabels: ["INBOX"],
    });
  }

  async permanentDelete(
    threadId: string,
    _messageIds: string[],
  ): Promise<void> {
    await invoke<void>("gmail_delete_thread", {
      accountId: this.accountId,
      threadId,
    });
  }

  async markRead(
    threadId: string,
    _messageIds: string[],
    read: boolean,
  ): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: read ? [] : ["UNREAD"],
      removeLabels: read ? ["UNREAD"] : [],
    });
  }

  async star(
    threadId: string,
    _messageIds: string[],
    starred: boolean,
  ): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: starred ? ["STARRED"] : [],
      removeLabels: starred ? [] : ["STARRED"],
    });
  }

  async spam(
    threadId: string,
    _messageIds: string[],
    isSpam: boolean,
  ): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: isSpam ? ["SPAM"] : ["INBOX"],
      removeLabels: isSpam ? ["INBOX"] : ["SPAM"],
    });
  }

  async moveToFolder(
    threadId: string,
    _messageIds: string[],
    folderPath: string,
  ): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: [folderPath],
      removeLabels: [],
    });
  }

  async addLabel(threadId: string, labelId: string): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: [labelId],
      removeLabels: [],
    });
  }

  async removeLabel(threadId: string, labelId: string): Promise<void> {
    await invoke<unknown>("gmail_modify_thread", {
      accountId: this.accountId,
      threadId,
      addLabels: [],
      removeLabels: [labelId],
    });
  }

  async sendMessage(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ id: string }> {
    const resp = await invoke<RustGmailSendResult>("gmail_send_email", {
      accountId: this.accountId,
      raw: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { id: resp.id };
  }

  async createDraft(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }> {
    const resp = await invoke<RustGmailDraft>("gmail_create_draft", {
      accountId: this.accountId,
      raw: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { draftId: resp.id };
  }

  async updateDraft(
    draftId: string,
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }> {
    const resp = await invoke<RustGmailDraft>("gmail_update_draft", {
      accountId: this.accountId,
      draftId,
      raw: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { draftId: resp.id };
  }

  async deleteDraft(draftId: string): Promise<void> {
    await invoke<void>("gmail_delete_draft", {
      accountId: this.accountId,
      draftId,
    });
  }

  async testConnection(): Promise<{ success: boolean; message: string }> {
    try {
      const profile = await invoke<RustGmailProfile>("gmail_test_connection", {
        accountId: this.accountId,
      });
      return {
        success: true,
        message: `Connected as ${profile.emailAddress}`,
      };
    } catch (err) {
      return {
        success: false,
        message:
          err instanceof Error ? err.message : String(err),
      };
    }
  }

  async getProfile(): Promise<{ email: string; name?: string | undefined }> {
    const profile = await invoke<RustGmailProfile>("gmail_test_connection", {
      accountId: this.accountId,
    });
    return { email: profile.emailAddress };
  }
}
