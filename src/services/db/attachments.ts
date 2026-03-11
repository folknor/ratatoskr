import { invoke } from "@tauri-apps/api/core";

export interface DbAttachment {
  id: string;
  message_id: string;
  account_id: string;
  filename: string | null;
  mime_type: string | null;
  size: number | null;
  gmail_attachment_id: string | null;
  content_id: string | null;
  is_inline: number;
  local_path: string | null;
  content_hash: string | null;
}

export async function upsertAttachment(att: {
  id: string;
  messageId: string;
  accountId: string;
  filename: string | null;
  mimeType: string | null;
  size: number | null;
  attachmentId: string | null;
  contentId: string | null;
  isInline: boolean;
}): Promise<void> {
  return invoke<void>("db_upsert_attachment", {
    id: att.id,
    messageId: att.messageId,
    accountId: att.accountId,
    filename: att.filename,
    mimeType: att.mimeType,
    size: att.size,
    attachmentId: att.attachmentId,
    contentId: att.contentId,
    isInline: att.isInline,
  });
}

export interface AttachmentWithContext {
  id: string;
  message_id: string;
  account_id: string;
  filename: string | null;
  mime_type: string | null;
  size: number | null;
  gmail_attachment_id: string | null;
  content_id: string | null;
  is_inline: number;
  local_path: string | null;
  content_hash: string | null;
  from_address: string | null;
  from_name: string | null;
  date: number | null;
  subject: string | null;
  thread_id: string | null;
}

export async function getAttachmentsForAccount(
  accountId: string,
  limit: number = 200,
  offset: number = 0,
): Promise<AttachmentWithContext[]> {
  return invoke<AttachmentWithContext[]>("db_get_attachments_for_account", {
    accountId,
    limit,
    offset,
  });
}

export interface AttachmentSender {
  from_address: string;
  from_name: string | null;
  count: number;
}

export async function getAttachmentSenders(
  accountId: string,
): Promise<AttachmentSender[]> {
  return invoke<AttachmentSender[]>("db_get_attachment_senders", {
    accountId,
  });
}

export async function getAttachmentsForMessage(
  accountId: string,
  messageId: string,
): Promise<DbAttachment[]> {
  return invoke<DbAttachment[]>("db_get_attachments_for_message", {
    accountId,
    messageId,
  });
}
