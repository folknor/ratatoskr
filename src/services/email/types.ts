import type { ParsedMessage } from "../gmail/messageParser";

export type AccountProvider =
  | "gmail_api"
  | "imap"
  | "caldav"
  | "jmap"
  | "graph";

export interface EmailFolder {
  id: string;
  name: string;
  path: string;
  type: "system" | "user";
  specialUse: string | null;
  delimiter: string;
  messageCount: number;
  unreadCount: number;
}

interface ProviderFolderBase {
  id: string;
  name: string;
  path: string;
  folderType: string;
  specialUse?: string | null;
  delimiter?: string | null;
  colorBg?: string | null;
  colorFg?: string | null;
}

export interface ProviderFolderListResult extends ProviderFolderBase {
  messageCount?: number | null;
  unreadCount?: number | null;
}

export interface ProviderFolderMutationResult extends ProviderFolderBase {}

export interface ProviderTestResult {
  success: boolean;
  message: string;
}

export interface ProviderProfile {
  email: string;
  name?: string;
}

export interface SyncResult {
  messages: ParsedMessage[];
  folderStatus?: {
    uidvalidity: number;
    lastUid: number;
    modseq?: number;
  };
  latestSyncToken?: string;
}

export interface EmailProvider {
  readonly accountId: string;
  readonly type: AccountProvider;

  // Folder/Label operations
  listFolders(): Promise<EmailFolder[]>;
  createFolder(name: string, parentPath?: string): Promise<EmailFolder>;
  deleteFolder(path: string): Promise<void>;
  renameFolder(path: string, newName: string): Promise<void>;

  // Sync operations
  initialSync(
    daysBack: number,
    onProgress?: (phase: string, current: number, total: number) => void,
  ): Promise<SyncResult>;
  deltaSync(syncToken: string): Promise<SyncResult>;

  // Message operations
  fetchMessage(messageId: string): Promise<ParsedMessage>;
  fetchAttachment(
    messageId: string,
    attachmentId: string,
  ): Promise<{ data: string; size: number }>;
  fetchRawMessage(messageId: string): Promise<string>;

  // Actions operate at thread scope. Message-level reply/forward lives elsewhere.
  // Gmail/JMAP/Graph route through Rust provider_* commands.
  archive?(threadId: string): Promise<void>;
  trash?(threadId: string): Promise<void>;
  permanentDelete?(threadId: string): Promise<void>;
  markRead?(threadId: string, read: boolean): Promise<void>;
  star?(threadId: string, starred: boolean): Promise<void>;
  spam?(threadId: string, isSpam: boolean): Promise<void>;
  moveToFolder?(threadId: string, folderPath: string): Promise<void>;
  addLabel?(threadId: string, labelId: string): Promise<void>;
  removeLabel?(threadId: string, labelId: string): Promise<void>;

  // Send/Draft operations
  // Only implemented by IMAP provider — Gmail/JMAP/Graph route through Rust provider_* commands.
  sendMessage?(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ id: string }>;
  createDraft?(
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }>;
  updateDraft?(
    draftId: string,
    rawBase64Url: string,
    threadId?: string,
  ): Promise<{ draftId: string }>;
  deleteDraft?(draftId: string): Promise<void>;

  // Connection
  testConnection(): Promise<{ success: boolean; message: string }>;
  getProfile(): Promise<{ email: string; name?: string | undefined }>;
}
