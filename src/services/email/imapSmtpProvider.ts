import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import type { EmailFolder, EmailProvider, SyncResult } from "./types";

interface ProviderTestResult {
  success: boolean;
  message: string;
}

interface ProviderProfile {
  email: string;
  name?: string;
}

interface ProviderAttachment {
  data: string;
  size: number;
}

interface ProviderFolder {
  id: string;
  name: string;
  path: string;
  folderType: string;
  specialUse?: string | null;
  delimiter?: string | null;
  messageCount?: number | null;
  unreadCount?: number | null;
}

/**
 * Thin IMAP adapter backed by unified Rust provider commands.
 */
export class ImapSmtpProvider implements EmailProvider {
  readonly accountId: string;
  readonly type = "imap" as const;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  clearConfigCache(): void {
    // No-op: IMAP config loading now lives in Rust.
  }

  async listFolders(): Promise<EmailFolder[]> {
    const folders = await invoke<ProviderFolder[]>("provider_list_folders", {
      accountId: this.accountId,
    });
    return folders.map((folder) => ({
      id: folder.id,
      name: folder.name,
      path: folder.path,
      type: folder.folderType === "system" ? "system" : "user",
      specialUse: folder.specialUse ?? null,
      delimiter: folder.delimiter ?? "/",
      messageCount: folder.messageCount ?? 0,
      unreadCount: folder.unreadCount ?? 0,
    }));
  }

  async createFolder(name: string, parentPath?: string): Promise<EmailFolder> {
    const folder = await invoke<ProviderFolder>("provider_create_folder", {
      accountId: this.accountId,
      name,
      parentId: parentPath ?? null,
    });
    return {
      id: folder.id,
      name: folder.name,
      path: folder.path,
      type: folder.folderType === "system" ? "system" : "user",
      specialUse: folder.specialUse ?? null,
      delimiter: folder.delimiter ?? "/",
      messageCount: folder.messageCount ?? 0,
      unreadCount: folder.unreadCount ?? 0,
    };
  }

  async deleteFolder(path: string): Promise<void> {
    await invoke("provider_delete_folder", {
      accountId: this.accountId,
      folderId: path,
    });
  }

  async renameFolder(path: string, newName: string): Promise<void> {
    await invoke("provider_rename_folder", {
      accountId: this.accountId,
      folderId: path,
      newName,
    });
  }

  async initialSync(
    _daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    throw new Error(
      "IMAP sync is handled by the Rust sync engine via syncManager. " +
        "Do not call initialSync() on ImapSmtpProvider directly.",
    );
  }

  async deltaSync(_syncToken: string): Promise<SyncResult> {
    throw new Error(
      "IMAP sync is handled by the Rust sync engine via syncManager. " +
        "Do not call deltaSync() on ImapSmtpProvider directly.",
    );
  }

  async fetchMessage(messageId: string): Promise<ParsedMessage> {
    return invoke<ParsedMessage>("provider_fetch_message", {
      accountId: this.accountId,
      messageId,
    });
  }

  async fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }> {
    return invoke<ProviderAttachment>("provider_fetch_attachment", {
      accountId: this.accountId,
      messageId,
      attachmentId,
    });
  }

  async fetchRawMessage(messageId: string): Promise<string> {
    return invoke<string>("provider_fetch_raw_message", {
      accountId: this.accountId,
      messageId,
    });
  }

  async archive(threadId: string, _messageIds: string[]): Promise<void> {
    await invoke("provider_archive", {
      accountId: this.accountId,
      threadId,
    });
  }

  async trash(threadId: string, _messageIds: string[]): Promise<void> {
    await invoke("provider_trash", {
      accountId: this.accountId,
      threadId,
    });
  }

  async permanentDelete(
    threadId: string,
    _messageIds: string[],
  ): Promise<void> {
    await invoke("provider_permanent_delete", {
      accountId: this.accountId,
      threadId,
    });
  }

  async markRead(
    threadId: string,
    _messageIds: string[],
    read: boolean,
  ): Promise<void> {
    await invoke("provider_mark_read", {
      accountId: this.accountId,
      threadId,
      read,
    });
  }

  async star(
    threadId: string,
    _messageIds: string[],
    starred: boolean,
  ): Promise<void> {
    await invoke("provider_star", {
      accountId: this.accountId,
      threadId,
      starred,
    });
  }

  async spam(
    threadId: string,
    _messageIds: string[],
    isSpam: boolean,
  ): Promise<void> {
    await invoke("provider_spam", {
      accountId: this.accountId,
      threadId,
      isSpam,
    });
  }

  async moveToFolder(
    threadId: string,
    _messageIds: string[],
    folderPath: string,
  ): Promise<void> {
    await invoke("provider_move_to_folder", {
      accountId: this.accountId,
      threadId,
      folderId: folderPath,
    });
  }

  async addLabel(threadId: string, labelId: string): Promise<void> {
    await invoke("provider_add_tag", {
      accountId: this.accountId,
      threadId,
      tagId: labelId,
    });
  }

  async removeLabel(threadId: string, labelId: string): Promise<void> {
    await invoke("provider_remove_tag", {
      accountId: this.accountId,
      threadId,
      tagId: labelId,
    });
  }

  async sendMessage(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ id: string }> {
    const id = await invoke<string>("provider_send_email", {
      accountId: this.accountId,
      rawBase64url: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { id };
  }

  async createDraft(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }> {
    const draftId = await invoke<string>("provider_create_draft", {
      accountId: this.accountId,
      rawBase64url: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { draftId };
  }

  async updateDraft(
    draftId: string,
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }> {
    const updatedDraftId = await invoke<string>("provider_update_draft", {
      accountId: this.accountId,
      draftId,
      rawBase64url: rawBase64Url,
      threadId: threadId ?? null,
    });
    return { draftId: updatedDraftId };
  }

  async deleteDraft(draftId: string): Promise<void> {
    await invoke("provider_delete_draft", {
      accountId: this.accountId,
      draftId,
    });
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
