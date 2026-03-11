import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import type {
  AccountProvider,
  EmailFolder,
  EmailProvider,
  SyncResult,
} from "./types";

interface JmapFolder {
  id: string;
  name: string;
  path: string;
  folderType: string;
  specialUse: string | null;
  messageCount: number;
  unreadCount: number;
}

interface ProviderFolderResult {
  id: string;
  name: string;
  path: string;
  folderType: string;
  specialUse: string | null;
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
 * EmailProvider adapter for JMAP accounts.
 * Delegates to Tauri jmap_* commands — Rust handles all protocol details.
 */
export class JmapProvider implements EmailProvider {
  readonly accountId: string;
  readonly type: AccountProvider = "jmap";

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  // ---- Folder/Label operations ----

  async listFolders(): Promise<EmailFolder[]> {
    const folders = await invoke<JmapFolder[]>("jmap_list_folders", {
      accountId: this.accountId,
    });

    return folders.map((f) => ({
      id: f.id,
      name: f.name,
      path: f.path,
      type: (f.folderType === "system" ? "system" : "user") as
        | "system"
        | "user",
      specialUse: f.specialUse,
      delimiter: "/",
      messageCount: f.messageCount,
      unreadCount: f.unreadCount,
    }));
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

  // ---- Sync operations ----

  async initialSync(
    daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    await invoke("jmap_sync_initial", {
      accountId: this.accountId,
      daysBack,
    });

    return { messages: [] };
  }

  async deltaSync(_syncToken: string): Promise<SyncResult> {
    const result = await invoke<{
      newInboxEmailIds: string[];
      affectedThreadIds: string[];
    }>("jmap_sync_delta", { accountId: this.accountId });

    const syncResult: SyncResult = { messages: [] };
    if (result.affectedThreadIds.length > 0) {
      syncResult.latestSyncToken = "updated";
    }
    return syncResult;
  }

  // ---- Message operations ----

  async fetchMessage(_messageId: string): Promise<ParsedMessage> {
    // JMAP sync writes bodies directly to the body store in Rust.
    // Per-message fetch is not needed; use body_store_get instead.
    throw new Error(
      "JMAP does not support per-message fetch. Message bodies are populated during sync.",
    );
  }

  async fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }> {
    return invoke<{ data: string; size: number }>("jmap_fetch_attachment", {
      accountId: this.accountId,
      emailId: messageId,
      blobId: attachmentId,
    });
  }

  async fetchRawMessage(_messageId: string): Promise<string> {
    throw new Error(
      "JMAP does not support raw message fetch in the current implementation.",
    );
  }

  // ---- Connection ----

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
