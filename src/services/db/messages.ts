import { bodyStorePut } from "@/core/rustDb";

import { getDb } from "./connection";

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
  const db = await getDb();
  return db.select<DbMessage[]>(
    "SELECT * FROM messages WHERE account_id = $1 AND thread_id = $2 ORDER BY date ASC",
    [accountId, threadId],
  );
}

export async function getMessagesByIds(
  accountId: string,
  messageIds: string[],
): Promise<DbMessage[]> {
  if (messageIds.length === 0) return [];
  const db = await getDb();
  // SQLite has a 999-parameter limit. Chunk to stay well under.
  const CHUNK = 500;
  const results: DbMessage[] = [];
  for (let i = 0; i < messageIds.length; i += CHUNK) {
    const chunk = messageIds.slice(i, i + CHUNK);
    const placeholders = chunk.map((_, j) => `$${j + 2}`).join(", ");
    const rows = await db.select<DbMessage[]>(
      `SELECT * FROM messages WHERE account_id = $1 AND id IN (${placeholders})`,
      [accountId, ...chunk],
    );
    results.push(...rows);
  }
  return results;
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
  const db = await getDb();

  const hasBody = !!(msg.bodyHtml || msg.bodyText);

  // Store bodies in the compressed body store first, then set body_cached = 1.
  // This ensures body_cached is only set after the body is actually persisted.
  if (hasBody) {
    await bodyStorePut(msg.id, msg.bodyHtml, msg.bodyText);
  }

  // Metadata DB — no longer stores body_html/body_text.
  // body_cached flag is set only after bodyStorePut succeeds above.
  await db.execute(
    `INSERT INTO messages (id, account_id, thread_id, from_address, from_name, to_addresses, cc_addresses, bcc_addresses, reply_to, subject, snippet, date, is_read, is_starred, body_cached, raw_size, internal_date, list_unsubscribe, list_unsubscribe_post, auth_results, message_id_header, references_header, in_reply_to_header, imap_uid, imap_folder)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)
     ON CONFLICT(account_id, id) DO UPDATE SET
       from_address = $4, from_name = $5, to_addresses = $6, cc_addresses = $7,
       bcc_addresses = $8, reply_to = $9, subject = $10, snippet = $11,
       date = $12, is_read = $13, is_starred = $14,
       body_cached = CASE WHEN $15 = 1 THEN 1 ELSE body_cached END,
       raw_size = $16, internal_date = $17, list_unsubscribe = $18, list_unsubscribe_post = $19,
       auth_results = $20, message_id_header = COALESCE($21, message_id_header),
       references_header = COALESCE($22, references_header),
       in_reply_to_header = COALESCE($23, in_reply_to_header),
       imap_uid = COALESCE($24, imap_uid), imap_folder = COALESCE($25, imap_folder)`,
    [
      msg.id,
      msg.accountId,
      msg.threadId,
      msg.fromAddress,
      msg.fromName,
      msg.toAddresses,
      msg.ccAddresses,
      msg.bccAddresses,
      msg.replyTo,
      msg.subject,
      msg.snippet,
      msg.date,
      msg.isRead ? 1 : 0,
      msg.isStarred ? 1 : 0,
      hasBody ? 1 : 0,
      msg.rawSize,
      msg.internalDate,
      msg.listUnsubscribe ?? null,
      msg.listUnsubscribePost ?? null,
      msg.authResults ?? null,
      msg.messageIdHeader ?? null,
      msg.referencesHeader ?? null,
      msg.inReplyToHeader ?? null,
      msg.imapUid ?? null,
      msg.imapFolder ?? null,
    ],
  );
}

export async function deleteMessage(
  accountId: string,
  messageId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM messages WHERE account_id = $1 AND id = $2", [
    accountId,
    messageId,
  ]);
}

export async function updateMessageThreadIds(
  accountId: string,
  messageIds: string[],
  threadId: string,
): Promise<void> {
  const db = await getDb();
  // SQLite variable limit is 999; process in chunks
  for (let i = 0; i < messageIds.length; i += 500) {
    const chunk = messageIds.slice(i, i + 500);
    const placeholders = chunk.map((_, idx) => `$${idx + 3}`).join(", ");
    await db.execute(
      `UPDATE messages SET thread_id = $1 WHERE account_id = $2 AND id IN (${placeholders})`,
      [threadId, accountId, ...chunk],
    );
  }
}

export async function deleteAllMessagesForAccount(
  accountId: string,
): Promise<void> {
  const db = await getDb();
  await db.execute("DELETE FROM messages WHERE account_id = $1", [accountId]);
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
  const db = await getDb();
  const messages = await db.select<DbMessage[]>(
    `SELECT * FROM messages
     WHERE account_id = $1 AND LOWER(from_address) = LOWER($2)
       AND body_cached = 1
     ORDER BY date DESC LIMIT $3`,
    [accountId, accountEmail, limit],
  );

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
