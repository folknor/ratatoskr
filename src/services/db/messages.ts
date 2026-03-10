import { invoke } from "@tauri-apps/api/core";

import { bodyStorePut } from "@/core/rustDb";

/**
 * Must match the `messages` CREATE TABLE schema (migrations.ts) and
 * the Rust `DbMessage` struct (src-tauri/src/db/types.rs).
 *
 * When fetched via direct SQL (getDb()), is_read/is_starred are 0|1 integers.
 * When fetched via Rust (db_get_messages_for_thread), they are booleans.
 * Both representations are truthy/falsy-compatible but strict === 1 checks
 * only work on the direct SQL path.
 */
export interface DbMessage {
  id: string;
  account_id: string;
  thread_id: string;
  from_address: string | null;
  from_name: string | null;
  to_addresses: string | null;
  cc_addresses: string | null;
  bcc_addresses: string | null;
  reply_to: string | null;
  subject: string | null;
  snippet: string | null;
  date: number;
  is_read: number;
  is_starred: number;
  body_html: string | null;
  body_text: string | null;
  body_cached: number;
  raw_size: number | null;
  internal_date: number | null;
  list_unsubscribe: string | null;
  list_unsubscribe_post: string | null;
  auth_results: string | null;
  message_id_header: string | null;
  references_header: string | null;
  in_reply_to_header: string | null;
  imap_uid: number | null;
  imap_folder: string | null;
}

export async function getMessagesForThread(
  accountId: string,
  threadId: string,
): Promise<DbMessage[]> {
  return invoke<DbMessage[]>("db_get_messages_for_thread", {
    accountId,
    threadId,
  });
}

export async function getMessagesByIds(
  accountId: string,
  messageIds: string[],
): Promise<DbMessage[]> {
  if (messageIds.length === 0) return [];
  return invoke<DbMessage[]>("db_get_messages_by_ids", {
    accountId,
    messageIds,
  });
}

export async function upsertMessage(msg: {
  id: string;
  accountId: string;
  threadId: string;
  fromAddress: string | null;
  fromName: string | null;
  toAddresses: string | null;
  ccAddresses: string | null;
  bccAddresses: string | null;
  replyTo: string | null;
  subject: string | null;
  snippet: string | null;
  date: number;
  isRead: boolean;
  isStarred: boolean;
  bodyHtml: string | null;
  bodyText: string | null;
  rawSize: number | null;
  internalDate: number | null;
  listUnsubscribe?: string | null;
  listUnsubscribePost?: string | null;
  authResults?: string | null;
  messageIdHeader?: string | null;
  referencesHeader?: string | null;
  inReplyToHeader?: string | null;
  imapUid?: number | null;
  imapFolder?: string | null;
}): Promise<void> {
  const hasBody = !!(msg.bodyHtml || msg.bodyText);

  // Store bodies in the compressed body store first, then set body_cached = 1.
  // This ensures body_cached is only set after the body is actually persisted.
  if (hasBody) {
    await bodyStorePut(msg.id, msg.bodyHtml, msg.bodyText);
  }

  // Metadata DB — no longer stores body_html/body_text.
  // body_cached flag is set only after bodyStorePut succeeds above.
  return invoke<void>("db_upsert_message", {
    id: msg.id,
    accountId: msg.accountId,
    threadId: msg.threadId,
    fromAddress: msg.fromAddress,
    fromName: msg.fromName,
    toAddresses: msg.toAddresses,
    ccAddresses: msg.ccAddresses,
    bccAddresses: msg.bccAddresses,
    replyTo: msg.replyTo,
    subject: msg.subject,
    snippet: msg.snippet,
    date: msg.date,
    isRead: msg.isRead,
    isStarred: msg.isStarred,
    bodyCached: hasBody,
    rawSize: msg.rawSize,
    internalDate: msg.internalDate,
    listUnsubscribe: msg.listUnsubscribe ?? null,
    listUnsubscribePost: msg.listUnsubscribePost ?? null,
    authResults: msg.authResults ?? null,
    messageIdHeader: msg.messageIdHeader ?? null,
    referencesHeader: msg.referencesHeader ?? null,
    inReplyToHeader: msg.inReplyToHeader ?? null,
    imapUid: msg.imapUid ?? null,
    imapFolder: msg.imapFolder ?? null,
  });
}

export async function deleteMessage(
  accountId: string,
  messageId: string,
): Promise<void> {
  return invoke<void>("db_delete_message", { accountId, messageId });
}

export async function updateMessageThreadIds(
  accountId: string,
  messageIds: string[],
  threadId: string,
): Promise<void> {
  if (messageIds.length === 0) return;
  return invoke<void>("db_update_message_thread_ids", {
    accountId,
    messageIds,
    threadId,
  });
}

export async function deleteAllMessagesForAccount(
  accountId: string,
): Promise<void> {
  return invoke<void>("db_delete_all_messages_for_account", { accountId });
}

/**
 * Get recent sent messages for an account, matching from_address to account email.
 * Used for writing style analysis. Hydrates bodies from body store.
 */
export async function getRecentSentMessages(
  accountId: string,
  accountEmail: string,
  limit: number = 15,
): Promise<DbMessage[]> {
  const messages = await invoke<DbMessage[]>("db_get_recent_sent_messages", {
    accountId,
    accountEmail,
    limit,
  });

  // Hydrate bodies from body store
  if (messages.length > 0) {
    const { bodyStoreGetBatch } = await import("@/core/rustDb");
    const ids = messages.map((m) => m.id);
    const bodies = await bodyStoreGetBatch(ids);
    const bodyMap = new Map(bodies.map((b) => [b.messageId, b]));

    for (const msg of messages) {
      const body = bodyMap.get(msg.id);
      if (body) {
        msg.body_html = body.bodyHtml;
        msg.body_text = body.bodyText;
      }
    }

    // Filter to messages that actually have body_text with content
    return messages.filter(
      (m) => m.body_text != null && m.body_text.length > 50,
    );
  }

  return messages;
}
