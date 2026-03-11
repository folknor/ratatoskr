import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import type {
  EmailFolder,
  EmailProvider,
  ProviderFolderResult,
  ProviderProfile,
  ProviderTestResult,
  SyncResult,
} from "./types";

export abstract class RustBackedProviderBase implements EmailProvider {
  abstract readonly accountId: string;
  abstract readonly type: EmailProvider["type"];

  abstract listFolders(): Promise<EmailFolder[]>;
  abstract initialSync(
    daysBack: number,
    onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult>;
  abstract deltaSync(syncToken: string): Promise<SyncResult>;
  abstract fetchMessage(messageId: string): Promise<ParsedMessage>;
  abstract fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }>;
  abstract fetchRawMessage(messageId: string): Promise<string>;

  clearConfigCache(): void {}

  // Thread actions for Rust-backed providers are routed through `emailActions.ts`
  // rather than these defaults, so these throw-only fallbacks act as guard rails.
  async archive(_threadId: string, _messageIds: string[]): Promise<void> {
    throw new Error("Archive is not supported for this provider.");
  }

  async trash(_threadId: string, _messageIds: string[]): Promise<void> {
    throw new Error("Trash is not supported for this provider.");
  }

  async permanentDelete(
    _threadId: string,
    _messageIds: string[],
  ): Promise<void> {
    throw new Error("Permanent delete is not supported for this provider.");
  }

  async markRead(
    _threadId: string,
    _messageIds: string[],
    _read: boolean,
  ): Promise<void> {
    throw new Error("Mark read is not supported for this provider.");
  }

  async star(
    _threadId: string,
    _messageIds: string[],
    _starred: boolean,
  ): Promise<void> {
    throw new Error("Star is not supported for this provider.");
  }

  async spam(
    _threadId: string,
    _messageIds: string[],
    _isSpam: boolean,
  ): Promise<void> {
    throw new Error("Spam actions are not supported for this provider.");
  }

  async moveToFolder(
    _threadId: string,
    _messageIds: string[],
    _folderPath: string,
  ): Promise<void> {
    throw new Error("Move to folder is not supported for this provider.");
  }

  async addLabel(_threadId: string, _labelId: string): Promise<void> {
    throw new Error("Add label is not supported for this provider.");
  }

  async removeLabel(_threadId: string, _labelId: string): Promise<void> {
    throw new Error("Remove label is not supported for this provider.");
  }

  async sendMessage(
    _rawBase64Url: string,
    _threadId?: string,
  ): Promise<{ id: string }> {
    throw new Error("Send message is not supported for this provider.");
  }

  async createDraft(
    _rawBase64Url: string,
    _threadId?: string,
  ): Promise<{ draftId: string }> {
    throw new Error("Create draft is not supported for this provider.");
  }

  async updateDraft(
    _draftId: string,
    _rawBase64Url: string,
    _threadId?: string,
  ): Promise<{ draftId: string }> {
    throw new Error("Update draft is not supported for this provider.");
  }

  async deleteDraft(_draftId: string): Promise<void> {
    throw new Error("Delete draft is not supported for this provider.");
  }

  async createFolder(name: string, parentPath?: string): Promise<EmailFolder> {
    const folder = await invoke<ProviderFolderResult>(
      "provider_create_folder",
      {
        accountId: this.accountId,
        name,
        parentId: parentPath ?? null,
        textColor: null,
        bgColor: null,
      },
    );
    return this.mapFolder(folder);
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
      textColor: null,
      bgColor: null,
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

  protected mapFolder(folder: ProviderFolderResult): EmailFolder {
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
}
