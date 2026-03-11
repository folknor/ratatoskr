export interface ParsedAttachment {
  filename: string;
  mimeType: string;
  size: number;
  attachmentId: string;
  contentId: string | null;
  isInline: boolean;
}

export interface ParsedMessage {
  id: string;
  threadId: string;
  fromAddress: string | null;
  fromName: string | null;
  toAddresses: string | null;
  ccAddresses: string | null;
  bccAddresses: string | null;
  replyTo: string | null;
  subject: string | null;
  snippet: string;
  date: number;
  isRead: boolean;
  isStarred: boolean;
  bodyHtml: string | null;
  bodyText: string | null;
  rawSize: number;
  internalDate: number;
  labelIds: string[];
  hasAttachments: boolean;
  attachments: ParsedAttachment[];
  listUnsubscribe: string | null;
  listUnsubscribePost: string | null;
  authResults: string | null;
}
