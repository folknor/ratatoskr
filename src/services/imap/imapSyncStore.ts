import { upsertAttachment } from "../db/attachments";
import { withSerializedExecution } from "../db/connection";
import { upsertMessage } from "../db/messages";
import { getPendingOpsForResource } from "../db/pendingOperations";
import { setThreadLabels, upsertThread } from "../db/threads";
import type { ParsedMessage } from "../gmail/messageParser";
import type { ThreadGroup } from "../threading/threadBuilder";
import type { ImapMessage } from "./tauriCommands";

import { THREAD_BATCH_SIZE } from "./imapSyncFetch";

// ---------------------------------------------------------------------------
// Thread storage
// ---------------------------------------------------------------------------

/**
 * Store threads and their messages into the local DB.
 */
// biome-ignore lint/complexity/useMaxParams: sync requires all these data maps
export async function storeThreadsAndMessages(
  accountId: string,
  threadGroups: ThreadGroup[],
  parsedByLocalId: Map<string, ParsedMessage>,
  imapMsgByLocalId: Map<string, ImapMessage>,
  labelsByRfcId?: Map<string, Set<string>>,
): Promise<ParsedMessage[]> {
  const storedMessages: ParsedMessage[] = [];

  // Pre-check pending ops OUTSIDE any transaction
  const skippedThreadIds = new Set<string>();
  for (const group of threadGroups) {
    const pendingOps = await getPendingOpsForResource(
      accountId,
      group.threadId,
    );
    if (pendingOps.length > 0) {
      console.log(
        `[imapSync] Skipping thread ${group.threadId}: has ${pendingOps.length} pending local ops`,
      );
      skippedThreadIds.add(group.threadId);
    }
  }

  // Process in batches within transactions to avoid long-held locks
  for (let i = 0; i < threadGroups.length; i += THREAD_BATCH_SIZE) {
    const batch = threadGroups.slice(i, i + THREAD_BATCH_SIZE);

    await withSerializedExecution(async () => {
      for (const group of batch) {
        if (skippedThreadIds.has(group.threadId)) continue;

        const messages = group.messageIds
          .map((id) => parsedByLocalId.get(id))
          .filter((m): m is ParsedMessage => m !== undefined);

        if (messages.length === 0) continue;

        // Assign threadId to each message
        for (const msg of messages) {
          msg.threadId = group.threadId;
        }

        // Sort by date ascending
        messages.sort((a, b) => a.date - b.date);

        const firstMessage = messages[0];
        const lastMessage = messages[messages.length - 1];
        if (!(firstMessage && lastMessage)) continue;

        // Collect all label IDs across messages in this thread.
        // Also include labels from duplicate folder copies (same RFC Message-ID
        // in multiple folders) that the threading algorithm may have deduplicated.
        const allLabelIds = new Set<string>();
        for (const msg of messages) {
          for (const lid of msg.labelIds) {
            allLabelIds.add(lid);
          }
          // Merge labels from all folder copies of this message
          const imapMsg = imapMsgByLocalId.get(msg.id);
          const rfcId = imapMsg?.message_id;
          if (rfcId && labelsByRfcId) {
            const extraLabels = labelsByRfcId.get(rfcId);
            if (extraLabels) {
              for (const lid of extraLabels) {
                allLabelIds.add(lid);
              }
            }
          }
        }

        const isRead = messages.every((m) => m.isRead);
        const isStarred = messages.some((m) => m.isStarred);
        const hasAttachments = messages.some((m) => m.hasAttachments);

        await upsertThread({
          id: group.threadId,
          accountId,
          subject: firstMessage.subject,
          snippet: lastMessage.snippet,
          lastMessageAt: lastMessage.date,
          messageCount: messages.length,
          isRead,
          isStarred,
          isImportant: false,
          hasAttachments,
        });

        const labelArray = [...allLabelIds];
        await setThreadLabels(accountId, group.threadId, labelArray);

        // Store messages sequentially to avoid concurrent DB writes
        for (const parsed of messages) {
          const imapMsg = imapMsgByLocalId.get(parsed.id);

          await upsertMessage({
            id: parsed.id,
            accountId,
            threadId: parsed.threadId,
            fromAddress: parsed.fromAddress,
            fromName: parsed.fromName,
            toAddresses: parsed.toAddresses,
            ccAddresses: parsed.ccAddresses,
            bccAddresses: parsed.bccAddresses,
            replyTo: parsed.replyTo,
            subject: parsed.subject,
            snippet: parsed.snippet,
            date: parsed.date,
            isRead: parsed.isRead,
            isStarred: parsed.isStarred,
            bodyHtml: parsed.bodyHtml,
            bodyText: parsed.bodyText,
            rawSize: parsed.rawSize,
            internalDate: parsed.internalDate,
            listUnsubscribe: parsed.listUnsubscribe,
            listUnsubscribePost: parsed.listUnsubscribePost,
            authResults: parsed.authResults,
            messageIdHeader: imapMsg?.message_id ?? null,
            referencesHeader: imapMsg?.references ?? null,
            inReplyToHeader: imapMsg?.in_reply_to ?? null,
            imapUid: imapMsg?.uid ?? null,
            imapFolder: imapMsg?.folder ?? null,
          });

          for (const att of parsed.attachments) {
            await upsertAttachment({
              id: `${parsed.id}_${att.gmailAttachmentId}`,
              messageId: parsed.id,
              accountId,
              filename: att.filename,
              mimeType: att.mimeType,
              size: att.size,
              gmailAttachmentId: att.gmailAttachmentId,
              contentId: att.contentId,
              isInline: att.isInline,
            });
          }

          storedMessages.push(parsed);
        }
      }
    });
  }

  return storedMessages;
}
