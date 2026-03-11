import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import { RustBackedProviderBase } from "./rustBackedProvider";
import type {
  AccountProvider,
  EmailFolder,
  ProviderFolderResult,
  SyncResult,
} from "./types";

/**
 * EmailProvider adapter for JMAP accounts.
 * Delegates to Tauri jmap_* commands — Rust handles all protocol details.
 */
export class JmapProvider extends RustBackedProviderBase {
  readonly accountId: string;
  readonly type: AccountProvider = "jmap";

  constructor(accountId: string) {
    super();
    this.accountId = accountId;
  }

  // ---- Folder/Label operations ----

  override async listFolders(): Promise<EmailFolder[]> {
    const folders = await invoke<ProviderFolderResult[]>(
      "provider_list_folders",
      {
        accountId: this.accountId,
      },
    );
    return folders.map((folder) => this.mapFolder(folder));
  }

  // ---- Sync operations ----

  override async initialSync(
    daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    await invoke("jmap_sync_initial", {
      accountId: this.accountId,
      daysBack,
    });

    return { messages: [] };
  }

  override async deltaSync(_syncToken: string): Promise<SyncResult> {
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

  override async fetchMessage(_messageId: string): Promise<ParsedMessage> {
    // JMAP sync writes bodies directly to the body store in Rust.
    // Per-message fetch is not needed; use body_store_get instead.
    throw new Error(
      "JMAP does not support per-message fetch. Message bodies are populated during sync.",
    );
  }

  override async fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }> {
    return invoke<{ data: string; size: number }>("jmap_fetch_attachment", {
      accountId: this.accountId,
      emailId: messageId,
      blobId: attachmentId,
    });
  }

  override async fetchRawMessage(_messageId: string): Promise<string> {
    throw new Error(
      "JMAP does not support raw message fetch in the current implementation.",
    );
  }
}
