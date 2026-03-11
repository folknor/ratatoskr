import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import { RustBackedProviderBase } from "./rustBackedProvider";
import type { EmailFolder, SyncResult } from "./types";

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
export class ImapSmtpProvider extends RustBackedProviderBase {
  readonly accountId: string;
  readonly type = "imap" as const;

  constructor(accountId: string) {
    super();
    this.accountId = accountId;
  }

  override clearConfigCache(): void {
    // No-op: IMAP config loading now lives in Rust.
  }

  override async listFolders(): Promise<EmailFolder[]> {
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

  override async initialSync(
    _daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    throw new Error(
      "IMAP sync is handled by the Rust sync engine via syncManager. " +
        "Do not call initialSync() on ImapSmtpProvider directly.",
    );
  }

  override async deltaSync(_syncToken: string): Promise<SyncResult> {
    throw new Error(
      "IMAP sync is handled by the Rust sync engine via syncManager. " +
        "Do not call deltaSync() on ImapSmtpProvider directly.",
    );
  }

  override async fetchMessage(messageId: string): Promise<ParsedMessage> {
    return invoke<ParsedMessage>("provider_fetch_message", {
      accountId: this.accountId,
      messageId,
    });
  }

  override async fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }> {
    return invoke<ProviderAttachment>("provider_fetch_attachment", {
      accountId: this.accountId,
      messageId,
      attachmentId,
    });
  }

  override async fetchRawMessage(messageId: string): Promise<string> {
    return invoke<string>("provider_fetch_raw_message", {
      accountId: this.accountId,
      messageId,
    });
  }

  override async archive(
    threadId: string,
    _messageIds: string[],
  ): Promise<void> {
    await invoke("provider_archive", {
      accountId: this.accountId,
      threadId,
    });
  }

  override async trash(threadId: string, _messageIds: string[]): Promise<void> {
    await invoke("provider_trash", {
      accountId: this.accountId,
      threadId,
    });
  }

  override async permanentDelete(
    threadId: string,
    _messageIds: string[],
  ): Promise<void> {
    await invoke("provider_permanent_delete", {
      accountId: this.accountId,
      threadId,
    });
  }

  override async markRead(
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

  override async star(
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

  override async spam(
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

  override async moveToFolder(
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

  override async addLabel(threadId: string, labelId: string): Promise<void> {
    await invoke("provider_add_tag", {
      accountId: this.accountId,
      threadId,
      tagId: labelId,
    });
  }

  override async removeLabel(threadId: string, labelId: string): Promise<void> {
    await invoke("provider_remove_tag", {
      accountId: this.accountId,
      threadId,
      tagId: labelId,
    });
  }

  override async sendMessage(
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

  override async createDraft(
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

  override async updateDraft(
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

  override async deleteDraft(draftId: string): Promise<void> {
    await invoke("provider_delete_draft", {
      accountId: this.accountId,
      draftId,
    });
  }
}
