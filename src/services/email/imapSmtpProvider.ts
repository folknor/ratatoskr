import { invoke } from "@tauri-apps/api/core";
import { type DbAccount, getAccount } from "../db/accounts";
import type { ParsedMessage } from "../gmail/messageParser";
import { getSyncableFolders, mapFolderToLabel } from "../imap/folderMapper";
import { buildImapConfig } from "../imap/imapConfigBuilder";
import { imapMessageToParsedMessage } from "../imap/imapSyncConvert";
import {
  type ImapConfig,
  imapFetchMessageBody,
  imapFetchRawMessage,
  imapListFolders,
} from "../imap/tauriCommands";
import { ensureFreshToken } from "../oauth/oauthTokenManager";
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
}

/**
 * Thin IMAP adapter.
 * Remaining direct IMAP calls are limited to folder listing and on-demand
 * message/raw fetch until the unified provider DTOs cover those cases.
 */
export class ImapSmtpProvider implements EmailProvider {
  readonly accountId: string;
  readonly type = "imap" as const;

  private imapConfig: ImapConfig | null = null;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  private async getAccount(): Promise<DbAccount> {
    const account = await getAccount(this.accountId);
    if (!account) {
      throw new Error(`Account ${this.accountId} not found`);
    }
    return account;
  }

  private async getLegacyImapConfig(): Promise<ImapConfig> {
    const account = await this.getAccount();
    if (account.auth_method === "oauth2") {
      const token = await ensureFreshToken(account);
      return buildImapConfig(account, token);
    }
    if (!this.imapConfig) {
      this.imapConfig = buildImapConfig(account);
    }
    return this.imapConfig;
  }

  clearConfigCache(): void {
    this.imapConfig = null;
  }

  async listFolders(): Promise<EmailFolder[]> {
    const config = await this.getLegacyImapConfig();
    const imapFolders = await imapListFolders(config);
    const syncable = getSyncableFolders(imapFolders);

    return syncable.map((folder) => {
      const mapping = mapFolderToLabel(folder);
      return {
        id: mapping.labelId,
        name: mapping.labelName,
        path: folder.path,
        type: mapping.type as "system" | "user",
        specialUse: folder.special_use,
        delimiter: folder.delimiter,
        messageCount: folder.exists,
        unreadCount: folder.unseen,
      };
    });
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
      delimiter: "/",
      messageCount: 0,
      unreadCount: 0,
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
    const { folder, uid } = this.parseImapMessageId(messageId);
    if (uid === null || !folder) {
      throw new Error(`Invalid IMAP message ID format: ${messageId}`);
    }

    const config = await this.getLegacyImapConfig();
    const imapMsg = await imapFetchMessageBody(config, folder, uid);
    const { parsed } = imapMessageToParsedMessage(
      imapMsg,
      this.accountId,
      folder,
    );
    parsed.id = messageId;
    return parsed;
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
    const { folder, uid } = this.parseImapMessageId(messageId);
    if (uid === null || !folder) {
      throw new Error(`Invalid IMAP message ID format: ${messageId}`);
    }

    const config = await this.getLegacyImapConfig();
    return imapFetchRawMessage(config, folder, uid);
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

  private parseImapMessageId(messageId: string): {
    folder: string | null;
    uid: number | null;
  } {
    const prefix = `imap-${this.accountId}-`;
    if (!messageId.startsWith(prefix)) {
      return { folder: null, uid: null };
    }

    const remainder = messageId.slice(prefix.length);
    const lastDash = remainder.lastIndexOf("-");
    if (lastDash === -1) {
      return { folder: null, uid: null };
    }

    const folder = remainder.slice(0, lastDash);
    const uid = parseInt(remainder.slice(lastDash + 1), 10);
    if (!folder || Number.isNaN(uid)) {
      return { folder: null, uid: null };
    }

    return { folder, uid };
  }
}
