import { invoke } from "@tauri-apps/api/core";
import type { ParsedMessage } from "../gmail/messageParser";
import { RustBackedProviderBase } from "./rustBackedProvider";
import type {
  AccountProvider,
  EmailFolder,
  ProviderFolderListResult,
  SyncResult,
} from "./types";

interface ProviderAttachment {
  data: string;
  size: number;
}

/**
 * Thin Graph adapter backed by unified Rust provider commands.
 */
export class GraphProvider extends RustBackedProviderBase {
  readonly accountId: string;
  readonly type: AccountProvider = "graph";

  constructor(accountId: string) {
    super();
    this.accountId = accountId;
  }

  override async listFolders(): Promise<EmailFolder[]> {
    const folders = await invoke<ProviderFolderListResult[]>(
      "provider_list_folders",
      {
        accountId: this.accountId,
      },
    );
    return folders.map((folder) => this.mapFolder(folder));
  }

  override async initialSync(
    _daysBack: number,
    _onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult> {
    throw new Error(
      "Graph sync is handled by the Rust sync engine via syncManager. " +
        "Do not call initialSync() on GraphProvider directly.",
    );
  }

  override async deltaSync(_syncToken: string): Promise<SyncResult> {
    throw new Error(
      "Graph sync is handled by the Rust sync engine via syncManager. " +
        "Do not call deltaSync() on GraphProvider directly.",
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
}
